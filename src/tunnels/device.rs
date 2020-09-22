use crate::config::QUOTA_RESET_INTERVAL;
use crate::lws::{RMessage, WMessage};
use crate::myrpc;
use crate::tlsserver::WSStreamInfo;
use crate::udpx::{LongLiveX, UdpXMgr};
use futures::compat::Compat01As03;
use futures::task::Waker;
use log::{error, info};
use std::cell::RefCell;
use std::rc::Rc;
use tokio::sync::mpsc::UnboundedSender;

pub type LongLiveUD = Rc<RefCell<UserDevice>>;

pub struct UserDevice {
    pub uuid: String,
    need_cfg_pull: bool,
    is_in_pulling: bool,
    pub quota_remain: usize,
    pub quota_per_second: usize,
    wait_tasks: Vec<Waker>,
    wait_tunnels: Vec<WSStreamInfo>,
    tm: super::LongLiveTM,

    pub my_tunnels_ids: Vec<usize>,
    udpx_mgr: LongLiveX,
}

impl UserDevice {
    pub fn new(uuid: String, has_grpc: bool, tm: super::LongLiveTM) -> LongLiveUD {
        let need_cfg_pull = has_grpc;
        let v = UserDevice {
            uuid,
            need_cfg_pull,
            is_in_pulling: false,
            quota_remain: 0,
            quota_per_second: 0,
            wait_tasks: Vec::with_capacity(20),
            wait_tunnels: Vec::with_capacity(20),
            my_tunnels_ids: Vec::with_capacity(20),
            tm,
            udpx_mgr: UdpXMgr::new(),
        };

        Rc::new(RefCell::new(v))
    }

    pub fn reset_quota(&mut self) {
        if !self.has_flowctl() {
            return;
        }

        self.quota_remain = self.quota_per_second * (QUOTA_RESET_INTERVAL as usize / 1000);
        if self.wait_tasks.len() > 0 {
            loop {
                match self.wait_tasks.pop() {
                    Some(t) => {
                        t.wake();
                    }
                    None => {
                        return;
                    }
                }
            }
        }
    }

    pub fn consume(&mut self, bytes_count: usize) -> usize {
        if self.quota_remain > bytes_count {
            self.quota_remain = self.quota_remain - bytes_count;
        } else {
            self.quota_remain = 0;
        }

        self.quota_remain
    }

    pub fn poll_tunnel_quota_with(
        &mut self,
        bytes_cosume: usize,
        waker: Waker,
    ) -> Result<bool, ()> {
        let remain = self.consume(bytes_cosume);

        if remain == 0 {
            self.wait_tasks.push(waker);
            return Ok(false);
        }

        Ok(true)
    }

    pub fn has_flowctl(&self) -> bool {
        return self.quota_per_second > 0;
    }

    pub fn set_need_pull(&mut self, need_pull: bool) {
        info!(
            "[UserDevice]set_need_pull, id:{}, need_pull:{}",
            self.uuid, need_pull
        );
        self.need_cfg_pull = need_pull;
    }

    pub fn set_new_quota_per_second(&mut self, bandwidth_limit_kbs: u64) {
        info!(
            "[UserDevice]set_new_quota_per_second, id:{}, bandwidth_limit_kbs:{}",
            self.uuid, bandwidth_limit_kbs
        );
        self.quota_per_second = (bandwidth_limit_kbs * 1000) as usize;
    }

    pub fn serve_tunnel_create(&mut self, wsinfo: WSStreamInfo, ll: LongLiveUD) {
        if self.need_cfg_pull {
            self.wait_tunnels.push(wsinfo);

            self.start_pull_cfg(ll);

            return;
        }

        super::serve_websocket(
            wsinfo,
            self,
            ll,
            self.tm.clone(),
        );
    }

    fn start_pull_cfg(&mut self, ll: LongLiveUD) {
        info!("[UserDevice]start_pull_cfg, id:{}", self.uuid);
        if self.is_in_pulling {
            return;
        }

        self.is_in_pulling = true;

        let client;
        {
            client = self.tm.borrow_mut().get_grpc_client();
        }

        let mut pull = myrpc::CfgPullRequest::new();
        pull.set_uuid(self.uuid.to_string());

        match client.pull_cfg_async(&pull) {
            Ok(async_receiver) => {
                let ll1 = ll.clone();
                let ll2 = ll.clone();
                let fut = async move {
                    match Compat01As03::new(async_receiver).await {
                        Ok(rsp) => {
                            if rsp.code != 0 {
                                error!(
                                    "[UserDevice]start_pull_cfg, error, server code:{}",
                                    rsp.code
                                );
                            }

                            let mut rf = ll1.borrow_mut();
                            rf.is_in_pulling = false;
                            rf.on_cfg_pull_completed(rsp, ll1.clone());
                        }
                        Err(e) => {
                            error!("[UserDevice]start_pull_cfg, grpc error:{}", e);
                            let mut rf = ll2.borrow_mut();
                            match e {
                                grpcio::Error::RpcFailure(s) => {
                                    if s.status == grpcio::RpcStatusCode::UNAVAILABLE {
                                        // notify tm, it's grpc client need to rebuild
                                        rf.tm.borrow_mut().invalid_grpc_client();
                                    }
                                }
                                _ => {}
                            }

                            rf.is_in_pulling = false;
                            rf.on_cfg_pull_failed();
                        }
                    }
                };
                tokio::task::spawn_local(fut);
            }
            Err(e) => {
                error!("[UserDevice]start_pull_cfg, grpc failed:{}", e);
                self.is_in_pulling = false;
                self.on_cfg_pull_failed();
            }
        }
    }

    fn on_cfg_pull_completed(&mut self, rsp: myrpc::CfgPullResult, ll: LongLiveUD) {
        info!("[UserDevice]on_cfg_pull_completed, id:{}", self.uuid);
        if rsp.code != 0 {
            error!(
                "[UserDevice]on_cfg_pull_completed, uuid:{}, hub server reject, code:{}",
                self.uuid, rsp.code
            );
            self.on_cfg_pull_failed();
            return;
        }

        self.need_cfg_pull = false;
        self.quota_per_second = (rsp.bandwidth_limit_kbs * 1000) as usize;
        self.quota_remain = self.quota_per_second * (QUOTA_RESET_INTERVAL as usize / 1000);
        loop {
            match self.wait_tunnels.pop() {
                Some(wsinfo) => {
                    super::serve_websocket(
                        wsinfo,
                        self,
                        ll.clone(),
                        self.tm.clone(),
                    );
                }
                None => {
                    return;
                }
            }
        }
    }

    fn on_cfg_pull_failed(&mut self) {
        info!("[UserDevice]on_cfg_pull_failed, id:{}", self.uuid);
        self.wait_tunnels.clear();
    }

    pub fn remember(&mut self, tunnel_id: usize) {
        self.my_tunnels_ids.push(tunnel_id);
    }

    pub fn forget(&mut self, tunnel_id: usize) {
        self.my_tunnels_ids.retain(|&x| x != tunnel_id);
    }

    pub fn on_udpx_north(&mut self, tunnel_tx: UnboundedSender<WMessage>, msg: RMessage) {
        // on_udp_proxy_north(&mut self, lx:LongLiveX, tunnel_tx: UnboundedSender<WMessage>, mut msg: RMessage)
        let udpx_mgr = &self.udpx_mgr;
        udpx_mgr
            .borrow_mut()
            .on_udp_proxy_north(udpx_mgr.clone(), tunnel_tx, msg);
    }
}
