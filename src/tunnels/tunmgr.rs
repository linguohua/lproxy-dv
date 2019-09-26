use super::Tunnel;
use crate::config::{TunCfg, KEEP_ALIVE_INTERVAL};
use failure::Error;
use log::{debug, error, info};
use std::cell::RefCell;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::rc::Rc;
use std::result::Result;
use std::time::{Duration, Instant};
use stream_cancel::{StreamExt, Trigger, Tripwire};
use tokio::prelude::*;
use tokio::runtime::current_thread;
use tokio::timer::Interval;

type LongLive = Rc<RefCell<TunMgr>>;
pub type LongLiveTM = LongLive;

pub struct TunMgr {
    pub is_dns_tun: bool,
    pub dns_server_addr: Option<SocketAddr>,
    tunnel_id: usize,
    tunnels_map: HashMap<usize, Rc<RefCell<Tunnel>>>,
    keepalive_trigger: Option<Trigger>,
    discarded: bool,
}

impl TunMgr {
    pub fn new(cfg: &TunCfg, dns: bool) -> LongLive {
        let dns_server_addr;
        if dns {
            // if dns_server_addr invalid, panic
            dns_server_addr = Some(cfg.dns_server_addr.parse().unwrap());
        } else {
            dns_server_addr = None;
        }

        Rc::new(RefCell::new(TunMgr {
            is_dns_tun: dns,
            tunnel_id: 0,
            tunnels_map: HashMap::new(),
            keepalive_trigger: None,
            discarded: false,
            dns_server_addr,
        }))
    }

    pub fn init(&mut self, s: LongLive) -> Result<(), Error> {
        self.start_keepalive_timer(s);
        Ok(())
    }

    pub fn next_tunnel_id(&mut self) -> usize {
        let id = self.tunnel_id;
        self.tunnel_id += 1;

        id
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

        self.send_pings();
    }

    fn quota_reset(&mut self) {
        let tunnels = &self.tunnels_map;
        for (_, t) in tunnels.iter() {
            let mut tun = t.borrow_mut();
            tun.reset_quota_interval();
        }
    }

    fn start_keepalive_timer(&mut self, s2: LongLive) {
        info!("[TunMgr]start_keepalive_timer");
        let (trigger, tripwire) = Tripwire::new();
        self.keepalive_trigger = Some(trigger);

        let mut ping_interval = 0;
        // tokio timer, every few seconds
        let task = Interval::new(Instant::now(), Duration::from_millis(KEEP_ALIVE_INTERVAL))
            .skip(1)
            .take_until(tripwire)
            .for_each(move |instant| {
                debug!("[TunMgr]keepalive timer fire; instant={:?}", instant);

                let mut rf = s2.borrow_mut();
                ping_interval += 1;
                if ping_interval % 3 == 0 {
                    rf.keepalive();
                } else {
                    rf.quota_reset();
                }

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
}
