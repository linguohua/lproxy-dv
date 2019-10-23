use super::Tunnel;
use crate::config::{EtcdConfig, ServerCfg, KEEPALIVE_INTERVAL, QUOTA_RESET_INTERVAL};
use crate::myrpc;
use crate::service::{BandwidthReport, BandwidthReportMap, Instruction, TxType};
use failure::Error;
use fnv::FnvHashMap as HashMap;
use grpcio::{ChannelBuilder, ChannelCredentialsBuilder, Environment};
use log::{debug, error, info};
use std::cell::RefCell;
use std::net::SocketAddr;
use std::rc::Rc;
use std::result::Result;
use std::sync::Arc;
use std::time::{Duration, Instant};
use stream_cancel::{StreamExt, Trigger, Tripwire};
use tokio::prelude::*;
use tokio::runtime::current_thread;
use tokio::timer::Interval;

type LongLive = Rc<RefCell<TunMgr>>;
pub type LongLiveTM = LongLive;

pub struct TunMgr {
    pub dns_server_addr: Option<SocketAddr>,
    tunnel_id: usize,
    tunnels_map: HashMap<usize, Rc<RefCell<Tunnel>>>,
    keepalive_trigger: Option<Trigger>,
    discarded: bool,
    ins_tx: TxType,
    pub token_key: String,
    account_map: HashMap<String, super::LongLiveUA>,
    grpc_addr: String,
    grpc_client: Option<Arc<myrpc::DeviceCfgPullClient>>,
}

impl TunMgr {
    pub fn new(cfg: &ServerCfg, etcdcfg: &EtcdConfig, ins_tx: TxType) -> LongLive {
        let dns_server_addr = Some(cfg.dns_server_addr.parse().unwrap());
        let token_key = cfg.token_key.to_string();
        Rc::new(RefCell::new(TunMgr {
            tunnel_id: 0,
            tunnels_map: HashMap::default(),
            account_map: HashMap::default(),
            keepalive_trigger: None,
            discarded: false,
            dns_server_addr,
            ins_tx,
            token_key,
            grpc_addr: etcdcfg.hub_grpc_addr.to_string(),
            grpc_client: None,
        }))
    }

    pub fn init(&mut self, s: LongLive) -> Result<(), Error> {
        self.start_keepalive_timer(s);
        Ok(())
    }

    pub fn update_etcd_cfg(&mut self, etcdcfg: &EtcdConfig) {
        info!("[TunMgr] TunMgr update update_etcd_cfg");
        self.grpc_addr = etcdcfg.hub_grpc_addr.to_string();
        self.grpc_client = None;
    }

    pub fn next_tunnel_id(&mut self) -> usize {
        let id = self.tunnel_id;
        self.tunnel_id += 1;

        id
    }

    pub fn invalid_grpc_client(&mut self) {
        self.grpc_client = None;
    }

    pub fn get_grpc_client(&mut self) -> Arc<myrpc::DeviceCfgPullClient> {
        if self.grpc_client.is_some() {
            return self.grpc_client.as_ref().unwrap().clone();
        }

        let cre = ChannelCredentialsBuilder::new().build();
        let grpc_e = Arc::new(Environment::new(1));
        let channel = ChannelBuilder::new(grpc_e).secure_connect(&self.grpc_addr, cre); // TODO
        let client = myrpc::DeviceCfgPullClient::new(channel);

        let client = Arc::new(client);
        let rt = client.clone();
        self.grpc_client = Some(client);

        rt
    }

    pub fn has_grpc(&self) -> bool {
        self.grpc_addr.len() > 0
    }

    pub fn stop(&mut self) {
        if self.discarded != false {
            error!("[TunMgr]stop, tunmgr is already discarded");

            return;
        }

        self.discarded = true;

        self.keepalive_trigger = None;

        for (_, v) in &self.tunnels_map {
            let v = v.borrow();
            v.close_rawfd();
        }
    }

    pub fn on_tunnel_created(&mut self, tun: Rc<RefCell<Tunnel>>) -> Result<(), ()> {
        let id = tun.borrow().tunnel_id;
        self.tunnels_map.insert(id, tun);

        Ok(())
    }

    pub fn on_tunnel_closed(&mut self, tun: Rc<RefCell<Tunnel>>) {
        let mut tun = tun.borrow_mut();
        let id = tun.tunnel_id;
        self.tunnels_map.remove(&id);

        tun.on_closed();
    }

    fn send_pings(&self) {
        let tunnels = &self.tunnels_map;
        for (_, t) in tunnels.iter() {
            let mut tun = t.borrow_mut();
            if !tun.send_ping() {
                tun.close_rawfd();
            }
        }
    }

    fn keepalive(&mut self) {
        if self.discarded != false {
            error!("[TunMgr]keepalive, tunmgr is discarded, not do keepalive");

            return;
        }

        self.collect_flow_and_report();
        self.send_pings();
    }

    fn collect_flow_and_report(&mut self) {
        let tunnels = &self.tunnels_map;
        if tunnels.len() < 1 {
            return;
        }

        let mut map: BandwidthReportMap = HashMap::default();
        for (_, t) in tunnels.iter() {
            let mut tun = t.borrow_mut();
            let send_bytes_counter = tun.send_bytes_counter;
            let recv_bytes_counter = tun.recv_bytes_counter;
            // clear
            tun.send_bytes_counter = 0;
            tun.recv_bytes_counter = 0;

            let uuid = &tun.account.borrow().uuid;
            if uuid.len() < 1 {
                continue;
            }

            match map.get_mut(uuid) {
                Some(ref mut t) => {
                    t.send = t.send + send_bytes_counter;
                    t.recv = t.recv + recv_bytes_counter;
                }
                None => {
                    let bw = BandwidthReport {
                        send: send_bytes_counter,
                        recv: recv_bytes_counter,
                    };
                    map.insert(uuid.to_string(), bw);
                }
            }
        }

        if map.len() > 0 {
            // send to service
            let ins = Instruction::ReportBandwidth(map);
            match self.ins_tx.unbounded_send(ins) {
                Err(e) => {
                    error!(
                        "[TunMgr]collect_flow_and_report, unbounded_send failed:{}",
                        e
                    );
                }
                _ => {}
            }
        }
    }

    fn quota_reset(&mut self) {
        let account_map = &self.account_map;
        for (_, a) in account_map.iter() {
            let mut a = a.borrow_mut();
            a.reset_quota();
        }
    }

    fn start_keepalive_timer(&mut self, s2: LongLive) {
        info!("[TunMgr]start_keepalive_timer");
        let (trigger, tripwire) = Tripwire::new();
        self.keepalive_trigger = Some(trigger);

        let mut ping_interval = 0;
        // tokio timer, every few seconds
        let task = Interval::new(Instant::now(), Duration::from_millis(QUOTA_RESET_INTERVAL))
            .skip(1)
            .take_until(tripwire)
            .for_each(move |instant| {
                debug!("[TunMgr]keepalive timer fire; instant={:?}", instant);

                let mut rf = s2.borrow_mut();
                ping_interval += 1;
                if ping_interval % (KEEPALIVE_INTERVAL / QUOTA_RESET_INTERVAL) == 0 {
                    rf.keepalive();
                }

                rf.quota_reset();

                Ok(())
            })
            .map_err(|e| {
                error!(
                    "[TunMgr]start_keepalive_timer interval errored; err={:?}",
                    e
                )
            })
            .then(|_| {
                info!("[TunMgr] keepalive timer future completed");
                Ok(())
            });;

        current_thread::spawn(task);
    }

    pub fn allocate_account(&mut self, uuid: &str, ll: LongLiveTM) -> super::LongLiveUA {
        match self.account_map.get(uuid) {
            Some(a) => {
                return a.clone();
            }
            None => {
                let v = super::UserAccount::new(uuid.to_string(), self.has_grpc(), ll);
                self.account_map.insert(uuid.to_string(), v.clone());

                return v;
            }
        }
    }
}
