use crate::config::{EtcdConfig, ServerCfg};
use crate::myrpc;
use crate::tlsserver::{Listener, WSStreamInfo};
use crate::tunnels;

use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use futures::prelude::*;
use log::{error, info};
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;
use futures::channel::oneshot;

pub enum SubServiceCtlCmd {
    Stop,
    TcpTunnel(WSStreamInfo),
    UpdateEtcdCfg(Arc<EtcdConfig>),
    Kickout(String),
    CfgChangeNotify(myrpc::CfgChangeNotify),
}

pub enum SubServiceType {
    Listener,
    TunMgr,
}

impl fmt::Display for SubServiceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s;
        match self {
            SubServiceType::Listener => s = "Listener",
            SubServiceType::TunMgr => s = "TunMgr",
        }
        write!(f, "({})", s)
    }
}

impl fmt::Display for SubServiceCtlCmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s;
        match self {
            SubServiceCtlCmd::Stop => s = "Stop",
            SubServiceCtlCmd::TcpTunnel(_) => s = "TcpTunnel",
            SubServiceCtlCmd::UpdateEtcdCfg(_) => s = "UpdateEtcdCfg",
            SubServiceCtlCmd::Kickout(_) => s = "Kickout",
            SubServiceCtlCmd::CfgChangeNotify(_) => s = "CfgChangeNotify",
        }
        write!(f, "({})", s)
    }
}

pub struct SubServiceCtl {
    pub handler: Option<std::thread::JoinHandle<()>>,
    pub ctl_tx: Option<UnboundedSender<SubServiceCtlCmd>>,
    pub sstype: SubServiceType,
}

pub struct TunMgrStub {
    pub ctl_tx: UnboundedSender<SubServiceCtlCmd>,
}

fn start_listener(
    cfg: Arc<ServerCfg>,
    etcdcfg: Arc<EtcdConfig>,
    r_tx: oneshot::Sender<bool>,
    tmstubs: Vec<TunMgrStub>,
) -> SubServiceCtl {
    info!("[SubService]start_listener, tm count:{}", tmstubs.len(),);

    let (tx, mut rx) = unbounded_channel();
    let handler = std::thread::spawn(move || {
        let mut basic_rt = tokio::runtime::Builder::new()
        .basic_scheduler()
        .enable_all()
        .build().unwrap();
        // let handle = rt.handle();
        let local = tokio::task::LocalSet::new();

        let fut = async move {
            let listener = Listener::new(&cfg, &etcdcfg, tmstubs);
            // thread code
            if let Err(e) = listener.borrow_mut().init(listener.clone()) {
                error!("[SubService]listener start failed:{}", e);

                r_tx.send(false).unwrap();
                return;
            }

            r_tx.send(true).unwrap();

            // wait control signals
            while let Some(cmd) = rx.next().await {
                match cmd {
                    SubServiceCtlCmd::Stop => {
                        let f = listener.clone();
                        f.borrow_mut().stop();
                        break;
                    }
                    SubServiceCtlCmd::UpdateEtcdCfg(cfg) => {
                        let f = listener.clone();
                        f.borrow_mut().update_etcd_cfg(&cfg);
                    }
                    _ => {
                        error!("[SubService]listener unknown ctl cmd:{}", cmd);
                    }
                }
            };
        };

        local.spawn_local(fut);
        basic_rt.block_on(local);
    });

    SubServiceCtl {
        handler: Some(handler),
        ctl_tx: Some(tx),
        sstype: SubServiceType::Listener,
    }
}

fn start_one_tunmgr(
    cfg: Arc<ServerCfg>,
    etcdcfg: Arc<EtcdConfig>,
    r_tx: oneshot::Sender<bool>,
    ins_tx: super::TxType,
) -> SubServiceCtl {
    let (tx, mut rx) = unbounded_channel();
    let handler = std::thread::spawn(move || {
        let mut basic_rt = tokio::runtime::Builder::new()
        .basic_scheduler()
        .enable_all()
        .build().unwrap();
        // let handle = rt.handle();
        let local = tokio::task::LocalSet::new();

        let fut = async move {
            let tunmgr = tunnels::TunMgr::new(&cfg, &etcdcfg, ins_tx);
            // thread code
            if let Err(e) = tunmgr.borrow_mut().init(tunmgr.clone()) {
                error!("[SubService]tunmgr start failed:{}", e);

                r_tx.send(true).unwrap();
                return;
            }

            r_tx.send(true).unwrap();

            // wait control signals
            while let Some(cmd) = rx.next().await {
                match cmd {
                    SubServiceCtlCmd::Stop => {
                        let f = tunmgr.clone();
                        f.borrow_mut().stop();
                        break;
                    }
                    SubServiceCtlCmd::TcpTunnel(t) => {
                        let ua;
                        {
                            ua = tunmgr
                                .borrow_mut()
                                .allocate_device(&t.uuid, tunmgr.clone());
                        }

                        ua.borrow_mut().serve_tunnel_create(t, ua.clone());
                    }
                    SubServiceCtlCmd::UpdateEtcdCfg(cfg) => {
                        let f = tunmgr.clone();
                        f.borrow_mut().on_update_etcd_cfg(&cfg);
                    }
                    SubServiceCtlCmd::Kickout(uuid) => {
                        let f = tunmgr.clone();
                        f.borrow_mut().on_kickout_uuid(uuid);
                    }
                    SubServiceCtlCmd::CfgChangeNotify(cfg) => {
                        let f = tunmgr.clone();
                        f.borrow_mut().on_device_cfg_changed(cfg);
                    } // _ => {
                      //     error!("[SubService]tunmgr unknown ctl cmd:{}", cmd);
                      // }
                }
            };
        };

        local.spawn_local(fut);
        basic_rt.block_on(local);
    });

    SubServiceCtl {
        handler: Some(handler),
        ctl_tx: Some(tx),
        sstype: SubServiceType::TunMgr,
    }
}

async fn to_future(
    rx: oneshot::Receiver<bool>,
    ctrl: SubServiceCtl,
) -> std::result::Result<SubServiceCtl, () > {
    match rx.await {
        Ok(v) => {
            match v {
                true => {
                    Ok(ctrl)
                }
                false => {
                    Err(())
                }
            }
        }
        Err(_) => {
            Err(())
        }
    }
}

type SubsctlVec = Rc<RefCell<Vec<SubServiceCtl>>>;

async fn start_tunmgr(
    cfg: std::sync::Arc<ServerCfg>,
    etcdcfg: Arc<EtcdConfig>,
    ins_tx: super::TxType,
) -> std::result::Result<SubsctlVec, SubsctlVec> {
    let cpus = num_cpus::get();
    let sv = stream::iter(vec![0; cpus]);
    let subservices = Rc::new(RefCell::new(Vec::new()));
    let subservices3 = subservices.clone();
    let failed = Rc::new(RefCell::new(false));
    let failed3 = failed.clone();

    let fut = sv
        .for_each(move |_| {
            let (tx, rx) = oneshot::channel();
            let failed2 = failed.clone();
            let subservices2 = subservices.clone();
            let cfgx = cfg.clone();
            let etcdcfgx = etcdcfg.clone();
            let ins_tx = ins_tx.clone();
            let ctl = async move {
                let ct = to_future(rx, start_one_tunmgr(cfgx, etcdcfgx, tx, ins_tx)).await;
                match ct {
                    Ok(c) => {
                        subservices2.borrow_mut().push(c);
                    }
                    _ => {
                        *failed2.borrow_mut() = true;
                    }
                }
            };

            ctl
        });

    fut.await;

    if *failed3.borrow() {
        Err(subservices3)
    } else {
        Ok(subservices3)
    }
}

pub async fn start_subservice(
    cfg: std::sync::Arc<ServerCfg>,
    etcdcfg: Arc<EtcdConfig>,
    ins_tx: super::TxType,
) -> std::result::Result<SubsctlVec,()> {
    let cfg2 = cfg.clone();

    let ff = async move {
        // start tunmgr first
        let subservices = start_tunmgr(cfg.clone(), etcdcfg.clone(), ins_tx.clone()).await?;

        // finally start listener
        let (tx, rx) = oneshot::channel();
        //let v = subservices.clone();
        let mut tcp_sss_vec = Vec::new();
        {
            let ss = subservices.borrow();
            for s in ss.iter() {
                let tx = s.ctl_tx.as_ref().unwrap().clone();
                tcp_sss_vec.push(TunMgrStub { ctl_tx: tx });
            }
        }

        match to_future(rx, start_listener(cfg2.clone(), etcdcfg, tx, tcp_sss_vec)).await {
            Ok(ctl) => subservices.borrow_mut().push(ctl),
            Err(_) => return Err(subservices),
        }

        Ok(subservices)
    };
 
    match ff.await {
        Ok(v) => Ok(v),
        Err(v) => {
            let vec_subservices = &mut v.borrow_mut();
            // WARNING: block current thread
            cleanup_subservices(vec_subservices);
            Err(())
        }
    }
}

pub fn cleanup_subservices(subservices: &mut Vec<SubServiceCtl>) {
    for s in subservices.iter_mut() {
        let cmd = super::subservice::SubServiceCtlCmd::Stop;
        match s.ctl_tx.as_ref().unwrap().send(cmd) {
            Err(e) => {
                info!("[SubService] send ctl {} cmd failed:{}", s.sstype, e);
            }
            _ => {}
        }
        s.ctl_tx = None;
    }

    // WARNING: thread block wait!
    for s in subservices.iter_mut() {
        info!("[SubService] try to jon handle, sstype:{}", s.sstype);
        let h = &mut s.handler;
        let h = h.take();
        h.unwrap().join().unwrap();

        info!("[SubService] jon handle completed, sstype:{}", s.sstype);
    }
}
