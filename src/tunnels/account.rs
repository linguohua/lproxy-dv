use crate::config::QUOTA_RESET_INTERVAL;
use crate::myrpc;
use crate::tlsserver::WSStreamInfo;
use futures::task::Task;
use log::error;
use std::cell::RefCell;
use std::rc::Rc;
use tokio::prelude::*;
use tokio::runtime::current_thread;

pub type LongLiveUA = Rc<RefCell<UserAccount>>;

pub struct UserAccount {
    pub uuid: String,
    need_cfg_pull: bool,
    is_in_pulling: bool,
    pub quota_remain: usize,
    pub quota_per_second: usize,
    wait_tasks: Vec<Task>,
    wait_tunnels: Vec<WSStreamInfo>,
    tm: super::LongLiveTM,

    pub my_tunnels_ids: Vec<usize>,
}

impl UserAccount {
    pub fn new(uuid: String, has_grpc: bool, tm: super::LongLiveTM) -> LongLiveUA {
        let need_cfg_pull = has_grpc;
        let v = UserAccount {
            uuid,
            need_cfg_pull,
            is_in_pulling: false,
            quota_remain: 0,
            quota_per_second: 0,
            wait_tasks: Vec::with_capacity(20),
            wait_tunnels: Vec::with_capacity(20),
            my_tunnels_ids: Vec::with_capacity(20),
            tm,
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
                        t.notify();
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

    pub fn poll_tunnel_quota_with(&mut self, bytes_cosume: usize) -> Result<bool, ()> {
        let remain = self.consume(bytes_cosume);

        if remain == 0 {
            self.wait_tasks.push(futures::task::current());
            return Ok(false);
        }

        Ok(true)
    }

    pub fn has_flowctl(&self) -> bool {
        return self.quota_per_second > 0;
    }

    pub fn set_need_pull(&mut self, need_pull: bool) {
        self.need_cfg_pull = need_pull;
    }

    pub fn set_new_quota_per_second(&mut self, bandwidth_limit_kbs: u64) {
        self.quota_per_second = (bandwidth_limit_kbs * 1000) as usize;
    }

    pub fn serve_tunnel_create(&mut self, wsinfo: WSStreamInfo, ll: LongLiveUA) {
        if self.need_cfg_pull {
            self.wait_tunnels.push(wsinfo);

            self.start_pull_cfg(ll);

            return;
        }

        super::serve_websocket(
            wsinfo,
            self.quota_per_second as u32,
            self,
            ll,
            self.tm.clone(),
        );
    }

    fn start_pull_cfg(&mut self, ll: LongLiveUA) {
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
                let fut = async_receiver
                    .and_then(move |rsp| {
                        if rsp.code != 0 {
                            error!(
                                "[UserAccount]start_pull_cfg, error, server code:{}",
                                rsp.code
                            );
                        }

                        let mut rf = ll1.borrow_mut();
                        rf.is_in_pulling = false;
                        rf.on_cfg_pull_completed(rsp, ll1.clone());
                        Ok(())
                    })
                    .map_err(move |e| {
                        error!("[UserAccount]start_pull_cfg, grpc error:{}", e);
                        let mut rf = ll2.borrow_mut();
                        match e {
                            grpcio::Error::RpcFailure(s) => {
                                if s.status == grpcio::RpcStatusCode::Unavailable {
                                    // notify tm, it's grpc client need to rebuild
                                    rf.tm.borrow_mut().invalid_grpc_client();
                                }
                            }
                            _ => {}
                        }

                        rf.is_in_pulling = false;
                        rf.on_cfg_pull_failed();
                        ()
                    });

                current_thread::spawn(fut);
            }
            Err(e) => {
                error!("[UserAccount]start_pull_cfg, grpc failed:{}", e);
                self.is_in_pulling = false;
                self.on_cfg_pull_failed();
            }
        }
    }

    fn on_cfg_pull_completed(&mut self, rsp: myrpc::CfgPullResult, ll: LongLiveUA) {
        if rsp.code != 0 {
            error!(
                "[UserAccount]on_cfg_pull_completed, uuid:{}, hub server reject, code:{}",
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
                        self.quota_per_second as u32,
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
        self.wait_tunnels.clear();
    }

    pub fn remember(&mut self, tunnel_id: usize) {
        self.my_tunnels_ids.push(tunnel_id);
    }

    pub fn forget(&mut self, tunnel_id: usize) {
        self.my_tunnels_ids.retain(|&x| x != tunnel_id);
    }
}
