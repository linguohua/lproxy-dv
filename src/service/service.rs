use crate::config::{self, SERVICE_MONITOR_INTERVAL};
use crate::myrpc;
use futures::sync::mpsc::UnboundedSender;
use log::{debug, error, info};
use std::fmt;
use std::time::{Duration, Instant};
use stream_cancel::{StreamExt, Trigger, Tripwire};
use tokio::prelude::*;
use tokio::runtime::current_thread;
use tokio::timer::{Delay, Interval};
const STATE_STOPPED: u8 = 0;
const STATE_STARTING: u8 = 1;
const STATE_RUNNING: u8 = 2;
const STATE_STOPPING: u8 = 3;
use super::{RpcServer, SubServiceCtl, SubServiceCtlCmd};
use fnv::FnvHashMap as HashMap;
use grpcio::{ChannelBuilder, ChannelCredentialsBuilder, Environment};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

type LongLive = Rc<RefCell<Service>>;

pub struct BandwidthReport {
    pub send: u64,
    pub recv: u64,
}

impl BandwidthReport {
    pub fn merge(&mut self, other: &BandwidthReport) {
        self.send = self.send + other.send;
        self.recv = self.recv + other.recv;
    }
}

pub type BandwidthReportMap = HashMap<String, BandwidthReport>;

pub enum Instruction {
    StartSubServices,
    Restart,
    LoadEtcdConfig,
    MonitorEtcdConfig,
    UpdateEtcdConfig,
    ServiceMonitor,

    ReportBandwidth(BandwidthReportMap),
}

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s;
        match self {
            Instruction::StartSubServices => s = "StartSubServices",
            Instruction::Restart => s = "Restart",
            Instruction::LoadEtcdConfig => s = "LoadEtcdConfig",
            Instruction::MonitorEtcdConfig => s = "MonitorEtcdConfig",
            Instruction::UpdateEtcdConfig => s = "UpdateEtcdConfig",
            Instruction::ServiceMonitor => s = "ServiceMonitor",
            Instruction::ReportBandwidth(_) => s = "ReportBandwidth",
        }
        write!(f, "({})", s)
    }
}

pub type TxType = UnboundedSender<Instruction>;

pub struct Service {
    state: u8,
    subservices: Vec<SubServiceCtl>,
    ins_tx: Option<TxType>,
    tuncfg: Option<std::sync::Arc<config::ServerCfg>>,
    instruction_trigger: Option<Trigger>,
    etcdcfg: Option<std::sync::Arc<config::EtcdConfig>>,
    monitor_trigger: Option<Trigger>,

    flow_map: BandwidthReportMap,
    grpc_client: Option<myrpc::BandwidthReportClient>,

    rpc_server: RpcServer,
}

impl Service {
    pub fn new(cfg: config::ServerCfg) -> LongLive {
        let rpcser = RpcServer::new(&cfg);
        Rc::new(RefCell::new(Service {
            subservices: Vec::new(),
            ins_tx: None,
            tuncfg: Some(std::sync::Arc::new(cfg)),
            instruction_trigger: None,
            state: 0,
            etcdcfg: None,
            monitor_trigger: None,
            flow_map: HashMap::default(),
            grpc_client: None,
            rpc_server: rpcser,
        }))
    }

    // start config monitor
    pub fn start(&mut self, s: LongLive) {
        if self.state == STATE_STOPPED {
            self.state = STATE_STARTING;

            self.rpc_server.start().unwrap();

            let (tx, rx) = futures::sync::mpsc::unbounded();
            let (trigger, tripwire) = Tripwire::new();
            self.save_instruction_trigger(trigger);

            let clone = s.clone();
            let fut = rx
                .take_until(tripwire)
                .for_each(move |ins| {
                    Service::process_instruction(clone.clone(), ins);
                    Ok(())
                })
                .then(|_| {
                    info!("[Service] instruction rx future completed");

                    Ok(())
                });

            self.save_tx(Some(tx));
            let server_cfg = self.tuncfg.as_ref().unwrap();
            if server_cfg.etcd_addr.len() > 0 {
                self.fire_instruction(Instruction::LoadEtcdConfig);
            } else {
                let etcdcfg = config::EtcdConfig {
                    tun_path: server_cfg.tun_path.to_string(),
                    hub_grpc_addr: String::default(),
                };

                self.save_etcd_cfg(etcdcfg);
                self.fire_instruction(Instruction::StartSubServices);
            }

            current_thread::spawn(fut);
        } else {
            panic!("[Service] start failed, state not stopped");
        }
    }

    fn do_load_cfg_from_etcd(s: LongLive) {
        let s2 = s.clone();
        let s3 = s.clone();
        let rf = s.borrow();
        let server_cfg = rf.tuncfg.as_ref().unwrap();
        let fut = config::EtcdConfig::load_from_etcd(&server_cfg);
        let fut = fut
            .and_then(move |etcdcfg| {
                info!("[Service] do_load_cfg_from_etcd ok, cfg:{}", etcdcfg);
                s2.borrow_mut().save_etcd_cfg(etcdcfg);
                s2.borrow_mut()
                    .fire_instruction(Instruction::StartSubServices);
                Ok(())
            })
            .map_err(|errors| {
                for n in errors.iter() {
                    info!("[Service] do_load_cfg_from_etcd failed:{}", n);
                }

                let seconds = 30;
                info!(
                    "[Service] do_load_cfg_from_etcd failed, retry {} seconds later",
                    seconds
                );
                Service::delay_post_instruction(s3, seconds, Instruction::LoadEtcdConfig);
            });

        current_thread::spawn(fut);
    }

    fn do_update_cfg_from_etcd(s: LongLive) {
        let s2 = s.clone();
        let s3 = s.clone();
        let rf = s.borrow();
        let server_cfg = rf.tuncfg.as_ref().unwrap();
        let fut = config::EtcdConfig::load_from_etcd(&server_cfg);
        let fut = fut
            .and_then(move |etcdcfg| {
                info!("[Service] do_update_cfg_from_etcd ok, cfg:{}", etcdcfg);

                let mut rf = s2.borrow_mut();
                rf.save_etcd_cfg(etcdcfg);

                rf.notify_listener_update_etcdcfg();
                // re-monitor
                rf.fire_instruction(Instruction::MonitorEtcdConfig);

                Ok(())
            })
            .map_err(move |errors| {
                for n in errors.iter() {
                    info!("[Service] do_update_cfg_from_etcd failed:{}", n);
                }

                s3.borrow_mut()
                    .fire_instruction(Instruction::MonitorEtcdConfig);

                ()
            });

        current_thread::spawn(fut);
    }

    fn notify_listener_update_etcdcfg(&self) {
        for ss in self.subservices.iter() {
            if ss.ctl_tx.is_some() {
                let cmd = SubServiceCtlCmd::UpdateEtcdCfg(self.etcdcfg.as_ref().unwrap().clone());
                match ss.ctl_tx.as_ref().unwrap().unbounded_send(cmd) {
                    Err(e) => {
                        error!("[Service] send update etcdcfg to listener failed:{}", e);
                    }
                    _ => {}
                }
            }
        }
    }

    fn save_etcd_cfg(&mut self, etcdcfg: config::EtcdConfig) {
        self.etcdcfg = Some(std::sync::Arc::new(etcdcfg));
        self.build_grpc_client();
    }

    fn build_grpc_client(&mut self) {
        let etcdcfg = self.etcdcfg.as_ref().unwrap();

        if etcdcfg.hub_grpc_addr.len() > 0 {
            info!(
                "[Service] save_etcd_cfg, build grpc client to:{}",
                etcdcfg.hub_grpc_addr
            );
            let cre = ChannelCredentialsBuilder::new().build();
            let e = Arc::new(Environment::new(1));
            let channel = ChannelBuilder::new(e).secure_connect(&etcdcfg.hub_grpc_addr, cre);
            let client = myrpc::BandwidthReportClient::new(channel);

            self.grpc_client = Some(client);
        } else {
            self.grpc_client = None;
        }
    }

    fn do_etcd_monitor(s: LongLive) {
        info!("[Service] do_etcd_monitor called");
        let s2 = s.clone();
        let s3 = s.clone();
        let rf = s.borrow();
        let server_cfg = rf.tuncfg.as_ref().unwrap();
        let fut = config::EtcdConfig::monitor_etcd_cfg(&server_cfg);
        let fut = fut
            .and_then(move |_| {
                info!("[Service] do_etcd_monitor ok, reload etcd config");
                s2.borrow_mut()
                    .fire_instruction(Instruction::UpdateEtcdConfig);
                Ok(())
            })
            .map_err(|e| {
                info!("[Service] do_etcd_monitor failed:{}, re-monitor later", e);

                // re-monitor again
                Service::delay_post_instruction(s3, 10, Instruction::MonitorEtcdConfig);
                ()
            });

        current_thread::spawn(fut);
    }

    fn do_write_etcd_instance_data(&self) {
        let server_cfg = self.tuncfg.as_ref().unwrap();
        let fut = config::etcd_write_instance_data(server_cfg);
        let fut = fut
            .and_then(move |_| {
                info!("[Service] do_write_etcd_instance_data ok");
                Ok(())
            })
            .map_err(|errors| {
                for n in errors.iter() {
                    info!("[Service] do_write_etcd_instance_data failed:{}", n);
                }

                ()
            });

        current_thread::spawn(fut);
    }

    fn do_update_etcd_instance_ttl(s: LongLive) {
        let rf = s.borrow();
        let server_cfg = rf.tuncfg.as_ref().unwrap();
        if server_cfg.etcd_addr.len() < 1 {
            return;
        }

        let fut = config::etcd_update_instance_ttl(server_cfg);
        let fut = fut.and_then(move |_| Ok(())).map_err(|errors| {
            for n in errors.iter() {
                info!("[Service] do_update_etcd_instance_ttl failed:{}", n);
            }

            ()
        });

        current_thread::spawn(fut);
    }

    fn do_service_monitor(s: LongLive) {
        Service::do_update_etcd_instance_ttl(s.clone());

        // report to grpc
        Service::do_flow_report(s);
    }

    fn do_flow_report(s: LongLive) {
        let s1 = s.clone();
        let mut rf = s.borrow_mut();

        let flow_map = &rf.flow_map;
        if flow_map.len() < 1 {
            return;
        }

        if rf.grpc_client.is_none() {
            // reset to new hashmap
            rf.flow_map = HashMap::default();
            return;
        }

        let mut report = myrpc::BandwidthStatistics::new();
        let bwu = report.mut_statistics();

        for (uuid, bw) in flow_map.iter() {
            let mut bu = myrpc::BandwidthUsage::new();
            bu.set_uuid(uuid.to_string());
            bu.set_recv_bytes(bw.recv);
            bu.set_send_bytes(bw.send);

            bwu.push(bu);
        }

        // reset to new hashmap
        rf.flow_map = HashMap::default();

        match rf.grpc_client.as_ref().unwrap().report_async(&report) {
            Ok(async_receiver) => {
                let fut = async_receiver
                    .and_then(|rsp| {
                        if rsp.code != 0 {
                            error!("[Service]do_flow_report, error, server code:{}", rsp.code);
                        }

                        Ok(())
                    })
                    .map_err(move |e| {
                        error!("[Service]do_flow_report, grpc error:{}", e);
                        match e {
                            grpcio::Error::RpcFailure(s) => {
                                if s.status == grpcio::RpcStatusCode::Unavailable {
                                    s1.borrow_mut().build_grpc_client();
                                }
                            }
                            _ => {}
                        }

                        ()
                    });

                current_thread::spawn(fut);
            }
            Err(e) => {
                error!("[Service]do_flow_report, grpc failed:{}", e);
            }
        }
    }

    fn do_bandwidth_merge(s: LongLive, mut bwm: BandwidthReportMap) {
        // info!("[Service]do_bandwidth_merge, len:{}", bwm.len());
        let mut rf = s.borrow_mut();
        for (uuid, t) in bwm.drain() {
            match rf.flow_map.get_mut(&uuid) {
                Some(ref mut exist) => {
                    exist.merge(&t);
                }
                None => {
                    rf.flow_map.insert(uuid, t);
                }
            }
        }
    }

    pub fn stop(&mut self) {
        info!("[Service]stop");
        if self.state != STATE_RUNNING {
            error!("[Service] do_restart failed, state not running");
            return;
        }

        self.state = STATE_STOPPING;

        // drop trigger will complete monitor future
        self.monitor_trigger = None;

        // drop trigger will completed instruction future
        self.instruction_trigger = None;

        super::cleanup_subservices(&mut self.subservices);

        self.subservices.clear();

        self.state = STATE_STOPPED;
    }

    pub fn restart(&mut self) {
        self.fire_instruction(Instruction::Restart);
    }

    fn process_instruction(s: LongLive, ins: Instruction) {
        match ins {
            Instruction::StartSubServices => {
                Service::do_start_subservices(s.clone());
            }
            Instruction::Restart => {
                Service::do_restart(s.clone());
            }
            Instruction::LoadEtcdConfig => {
                Service::do_load_cfg_from_etcd(s.clone());
            }
            Instruction::MonitorEtcdConfig => {
                Service::do_etcd_monitor(s.clone());
            }
            Instruction::UpdateEtcdConfig => {
                Service::do_update_cfg_from_etcd(s.clone());
            }
            Instruction::ServiceMonitor => {
                Service::do_service_monitor(s.clone());
            }
            Instruction::ReportBandwidth(bw) => {
                Service::do_bandwidth_merge(s, bw);
            }
        }
    }

    fn delay_post_instruction(s: LongLive, seconds: u64, ins: Instruction) {
        info!(
            "[Service]delay_post_instruction, seconds:{}, ins:{}",
            seconds, ins
        );

        // delay 5 seconds
        let when = Instant::now() + Duration::from_millis(seconds * 1000);
        let task = Delay::new(when)
            .and_then(move |_| {
                debug!("[Service]delay_post_instruction retry...");
                s.borrow().fire_instruction(ins);

                Ok(())
            })
            .map_err(|e| {
                error!(
                    "[Service]delay_post_instruction delay retry errored, err={:?}",
                    e
                );
                ()
            });

        current_thread::spawn(task);
    }

    fn do_start_subservices(s: LongLive) {
        info!("[Service]do_start_subservices");
        let cfg;
        {
            cfg = s.borrow().tuncfg.as_ref().unwrap().clone();
        }

        let etcdcfg;
        {
            etcdcfg = s.borrow().etcdcfg.as_ref().unwrap().clone();
        }

        let ins_tx;
        {
            ins_tx = s.borrow().ins_tx.as_ref().unwrap().clone();
        }

        let clone = s.clone();
        let clone2 = s.clone();
        let has_etcd = cfg.etcd_addr.len() > 0;

        let fut = super::start_subservice(cfg, etcdcfg, ins_tx)
            .and_then(move |subservices| {
                let s2 = &mut clone.borrow_mut();
                let vec_subservices = &mut subservices.borrow_mut();
                while let Some(ctl) = vec_subservices.pop() {
                    s2.subservices.push(ctl);
                }

                s2.state = STATE_RUNNING;

                s2.start_monitor_timer(clone.clone());

                if has_etcd {
                    // start to monitor etcd
                    s2.do_write_etcd_instance_data();
                    s2.fire_instruction(Instruction::MonitorEtcdConfig);
                }

                Ok(())
            })
            .or_else(|_| {
                Service::delay_post_instruction(clone2, 50, Instruction::StartSubServices);
                Err(())
            });

        current_thread::spawn(fut);
    }

    fn do_restart(s1: LongLive) {
        info!("[Service]do_restart");
        let mut s = s1.borrow_mut();
        s.stop();
        s.start(s1.clone());
    }

    fn save_tx(&mut self, tx: Option<TxType>) {
        self.ins_tx = tx;
    }

    fn save_instruction_trigger(&mut self, trigger: Trigger) {
        self.instruction_trigger = Some(trigger);
    }

    fn fire_instruction(&self, ins: Instruction) {
        debug!("[Service]fire_instruction, ins:{}", ins);
        let tx = &self.ins_tx;
        match tx.as_ref() {
            Some(ref tx) => {
                if let Err(e) = tx.unbounded_send(ins) {
                    error!("[Service]fire_instruction failed:{}", e);
                }
            }
            None => {
                error!("[Service]fire_instruction failed: no tx");
            }
        }
    }

    fn save_monitor_trigger(&mut self, trigger: Trigger) {
        self.monitor_trigger = Some(trigger);
    }

    fn start_monitor_timer(&mut self, s2: LongLive) {
        info!("[Service]start_monitor_timer");
        let (trigger, tripwire) = Tripwire::new();
        self.save_monitor_trigger(trigger);

        // tokio timer, every 3 seconds
        let task = Interval::new(
            Instant::now(),
            Duration::from_millis(SERVICE_MONITOR_INTERVAL),
        )
        .skip(1)
        .take_until(tripwire)
        .for_each(move |instant| {
            debug!("[Service]monitor timer fire; instant={:?}", instant);

            let rf = s2.borrow_mut();
            rf.fire_instruction(Instruction::ServiceMonitor);

            Ok(())
        })
        .map_err(|e| error!("[Service]start_monitor_timer interval errored; err={:?}", e))
        .then(|_| {
            info!("[Service] monitor timer future completed");
            Ok(())
        });;

        current_thread::spawn(task);
    }
}
