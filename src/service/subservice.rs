use crate::config::ServerCfg;
use crate::tlsserver::{Listener, WSStreamInfo};
use crate::tunnels;
use futures::future::lazy;
use futures::stream::iter_ok;
use futures::stream::Stream;
use futures::sync::mpsc::{unbounded, UnboundedSender};
use futures::Future;
use log::{error, info};
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;
use tokio::runtime::current_thread::{self, Runtime};

pub enum SubServiceCtlCmd {
    Stop,
    TcpTunnel(WSStreamInfo),
}

impl fmt::Display for SubServiceCtlCmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s;
        match self {
            SubServiceCtlCmd::Stop => s = "Stop",
            SubServiceCtlCmd::TcpTunnel(_) => s = "TcpTunnel",
        }
        write!(f, "({})", s)
    }
}

pub struct SubServiceCtl {
    pub handler: Option<std::thread::JoinHandle<()>>,
    pub ctl_tx: Option<UnboundedSender<SubServiceCtlCmd>>,
}

pub struct TunMgrStub {
    pub ctl_tx: UnboundedSender<SubServiceCtlCmd>,
}

fn start_listener(
    cfg: Arc<ServerCfg>,
    r_tx: futures::Complete<bool>,
    dnstmsstubs: Vec<TunMgrStub>,
    tmstubs: Vec<TunMgrStub>,
) -> SubServiceCtl {
    info!(
        "[SubService]start_listener, tm count:{}, dns tm count:{}",
        tmstubs.len(),
        dnstmsstubs.len()
    );

    let (tx, rx) = unbounded();
    let handler = std::thread::spawn(move || {
        let mut rt = Runtime::new().unwrap();
        let fut = lazy(move || {
            let listener = Listener::new(&cfg, dnstmsstubs, tmstubs);
            // thread code
            if let Err(e) = listener.borrow_mut().init(listener.clone()) {
                error!("[SubService]listener start failed:{}", e);

                r_tx.send(false).unwrap();
                return Ok(());
            }

            r_tx.send(true).unwrap();

            // wait control signals
            let fut = rx.for_each(move |cmd| {
                match cmd {
                    SubServiceCtlCmd::Stop => {
                        let f = listener.clone();
                        f.borrow_mut().stop();
                    }
                    _ => {
                        error!("[SubService]listener unknown ctl cmd:{}", cmd);
                    }
                }

                Ok(())
            });

            current_thread::spawn(fut);

            Ok(())
        });

        rt.spawn(fut);
        rt.run().unwrap();
    });

    SubServiceCtl {
        handler: Some(handler),
        ctl_tx: Some(tx),
    }
}

fn start_one_tunmgr(
    cfg: Arc<ServerCfg>,
    r_tx: futures::Complete<bool>,
    dns: bool,
) -> SubServiceCtl {
    let (tx, rx) = unbounded();
    let handler = std::thread::spawn(move || {
        let mut rt = Runtime::new().unwrap();
        let fut = lazy(move || {
            let tunmgr = tunnels::TunMgr::new(&cfg, dns);
            // thread code
            if let Err(e) = tunmgr.borrow_mut().init(tunmgr.clone()) {
                error!("[SubService]tunmgr start failed:{}", e);

                r_tx.send(true).unwrap();
                return Ok(());
            }

            r_tx.send(true).unwrap();

            // wait control signals
            let fut = rx.for_each(move |cmd| {
                match cmd {
                    SubServiceCtlCmd::Stop => {
                        let f = tunmgr.clone();
                        f.borrow_mut().stop();
                    }
                    SubServiceCtlCmd::TcpTunnel(t) => {
                        tunnels::serve_websocket(t, tunmgr.clone());
                    } // _ => {
                      //     error!("[SubService]tunmgr unknown ctl cmd:{}", cmd);
                      // }
                }

                Ok(())
            });

            current_thread::spawn(fut);

            Ok(())
        });

        rt.spawn(fut);
        rt.run().unwrap();
    });

    SubServiceCtl {
        handler: Some(handler),
        ctl_tx: Some(tx),
    }
}

fn to_future(
    rx: futures::Oneshot<bool>,
    ctrl: SubServiceCtl,
) -> impl Future<Item = SubServiceCtl, Error = ()> {
    let fut = rx
        .and_then(|v| if v { Ok(ctrl) } else { Err(futures::Canceled) })
        .or_else(|_| Err(()));

    fut
}

type SubsctlVec = Rc<RefCell<Vec<SubServiceCtl>>>;

fn start_tunmgr(
    cfg: std::sync::Arc<ServerCfg>,
    dns: bool,
) -> impl Future<Item = SubsctlVec, Error = ()> {
    let cpus;
    if dns {
        cpus = 2;
    } else {
        cpus = num_cpus::get();
    }

    let stream = iter_ok::<_, ()>(vec![0; cpus]);
    let subservices = Rc::new(RefCell::new(Vec::new()));
    let subservices2 = subservices.clone();
    let subservices3 = subservices.clone();

    let fut = stream
        .for_each(move |_| {
            let (tx, rx) = futures::oneshot();
            let subservices = subservices.clone();
            to_future(rx, start_one_tunmgr(cfg.clone(), tx, dns)).and_then(move |ctl| {
                subservices.borrow_mut().push(ctl);
                Ok(())
            })
        })
        .and_then(move |_| Ok(subservices2))
        .or_else(move |_| {
            let vec_subservices = &mut subservices3.borrow_mut();
            // WARNING: block current thread
            cleanup_subservices(vec_subservices);

            Err(())
        });

    fut
}

pub fn start_subservice(
    cfg: std::sync::Arc<ServerCfg>,
) -> impl Future<Item = SubsctlVec, Error = ()> {
    let cfg2 = cfg.clone();

    // start tunmgr first
    let tunmgr_fut = start_tunmgr(cfg.clone(), false);

    let dns_tunmgr_fut = tunmgr_fut.and_then(move |subservices| {
        let v = subservices.clone();
        let fut = start_tunmgr(cfg.clone(), true)
            .and_then(|sss| Ok((subservices, sss)))
            .or_else(move |_| {
                let vec_subservices = &mut v.borrow_mut();
                // WARNING: block current thread
                cleanup_subservices(vec_subservices);
                Err(())
            });

        fut
    });

    // finally start listener
    let forward_fut = dns_tunmgr_fut.and_then(move |(tcp_sss, dns_sss)| {
        let (tx, rx) = futures::oneshot();
        //let v = subservices.clone();
        let mut tcp_sss_vec = Vec::new();
        {
            let ss = tcp_sss.borrow();
            for s in ss.iter() {
                let tx = s.ctl_tx.as_ref().unwrap().clone();
                tcp_sss_vec.push(TunMgrStub { ctl_tx: tx });
            }
        }

        let mut dns_sss_vec = Vec::new();
        {
            let ss = dns_sss.borrow();
            for s in ss.iter() {
                let tx = s.ctl_tx.as_ref().unwrap().clone();
                dns_sss_vec.push(TunMgrStub { ctl_tx: tx });
            }
        }

        let mut v = Vec::new();
        {
            let mut ss = tcp_sss.borrow_mut();
            while let Some(p) = ss.pop() {
                v.push(p);
            }

            let mut ss = dns_sss.borrow_mut();
            while let Some(p) = ss.pop() {
                v.push(p);
            }
        }

        let v = Rc::new(RefCell::new(v));
        let v2 = v.clone();
        to_future(
            rx,
            start_listener(cfg2.clone(), tx, dns_sss_vec, tcp_sss_vec),
        )
        .and_then(|ctl| {
            v.borrow_mut().push(ctl);

            Ok(v)
        })
        .or_else(move |_| {
            let vec_subservices = &mut v2.borrow_mut();
            // WARNING: block current thread
            cleanup_subservices(vec_subservices);
            Err(())
        })
    });

    forward_fut
}

pub fn cleanup_subservices(subservices: &mut Vec<SubServiceCtl>) {
    for s in subservices.iter_mut() {
        let cmd = super::subservice::SubServiceCtlCmd::Stop;
        s.ctl_tx.as_ref().unwrap().unbounded_send(cmd).unwrap();
        s.ctl_tx = None;
    }

    // WARNING: thread block wait!
    for s in subservices.iter_mut() {
        let h = &mut s.handler;
        let h = h.take();
        h.unwrap().join().unwrap();

        info!("[SubService] jon handle completed");
    }
}
