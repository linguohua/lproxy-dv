use super::{Cmd, Reqq, THeader, THEADER_SIZE};
use crate::config::KEEP_ALIVE_INTERVAL;
use byte::*;
use bytes::Bytes;
use bytes::BytesMut;
use futures::sync::mpsc::UnboundedSender;
use log::{error, info};
use nix::sys::socket::{shutdown, Shutdown};
use std::cell::RefCell;
use std::net::SocketAddr;
use std::os::unix::io::RawFd;
use std::rc::Rc;
use std::result::Result;
use std::time::Instant;
use stream_cancel::Trigger;
use tungstenite::protocol::Message;

pub type LongLiveTun = Rc<RefCell<Tunnel>>;

pub struct Tunnel {
    pub tunnel_id: usize,
    pub tx: UnboundedSender<(u16, u16, Message)>,
    rawfd: RawFd,
    is_for_dns: bool,

    ping_count: u8,

    time: Instant,

    rtt_queue: Vec<i64>,
    rtt_index: usize,
    rtt_sum: i64,

    req_count: u16,
    pub requests: Reqq,

    pub dns_server_addr: Option<SocketAddr>,
}

impl Tunnel {
    pub fn new(
        tid: usize,
        tx: UnboundedSender<(u16, u16, Message)>,
        rawfd: RawFd,
        is_for_dns: bool,
    ) -> LongLiveTun {
        info!("[Tunnel]new Tunnel, idx:{}", tid);
        let size = 5;
        let rtt_queue = vec![0; size];

        Rc::new(RefCell::new(Tunnel {
            tunnel_id: tid,
            tx,
            rawfd,
            is_for_dns,
            ping_count: 0,
            time: Instant::now(),

            rtt_queue: rtt_queue,
            rtt_index: 0,
            rtt_sum: 0,

            req_count: 0,

            requests: Reqq::new(5),
            dns_server_addr: None,
        }))
    }

    pub fn on_tunnel_msg(&mut self, msg: Message, tl: LongLiveTun) {
        // info!("[Tunnel]on_tunnel_msg");
        if msg.is_pong() {
            self.on_pong(msg);

            return;
        }

        if !msg.is_binary() {
            info!("[Tunnel]tunnel should only handle binary msg!");
            return;
        }

        if self.is_for_dns {
            self.on_tunnel_dns_msg(msg, tl);
        } else {
            self.on_tunnel_proxy_msg(msg, tl);
        }
    }

    fn on_tunnel_dns_msg(&mut self, msg: Message, tl: LongLiveTun) {
        // do udp query
        let mut bs = msg.into_data();
        let offset = &mut 0;
        let port = bs.read_with::<u16>(offset, LE).unwrap();
        let ip32 = bs.read_with::<u32>(offset, LE).unwrap();

        super::proxy_dns(self, tl, bs.split_off(6), port, ip32);
    }

    fn on_tunnel_proxy_msg(&mut self, msg: Message, tl: LongLiveTun) {
        let bs = msg.into_data();
        let th = THeader::read_from(&bs[..]);
        let cmd = Cmd::from(th.cmd);
        match cmd {
            Cmd::ReqData => {
                // data
                let req_idx = th.req_idx;
                let req_tag = th.req_tag;
                let tx = self.get_request_tx(req_idx, req_tag);
                match tx {
                    None => {
                        info!("[Tunnel]no request found for: {}:{}", req_idx, req_tag);
                        return;
                    }
                    Some(tx) => {
                        let b = Bytes::from(&bs[THEADER_SIZE..]);
                        let result = tx.unbounded_send(b);
                        match result {
                            Err(e) => {
                                info!("[Tunnel]tunnel msg send to request failed:{}", e);
                                return;
                            }
                            _ => {}
                        }
                    }
                }
            }
            Cmd::ReqServerFinished => {
                // server finished
                let req_idx = th.req_idx;
                let req_tag = th.req_tag;
                self.free_request_tx(req_idx, req_tag);
            }
            Cmd::ReqServerClosed => {
                // server finished
                let req_idx = th.req_idx;
                let req_tag = th.req_tag;
                let reqs = &mut self.requests;
                let r = reqs.free(req_idx, req_tag);
                if r {
                    self.req_count -= 1;
                }
            }
            Cmd::ReqCreated => {
                let req_idx = th.req_idx;
                let req_tag = th.req_tag;
                let b = Bytes::from(&bs[THEADER_SIZE..]);
                let offset = &mut 0;
                let ip = b.read_with::<u32>(offset, LE).unwrap(); // 4 bytes
                let port = b.read_with::<u16>(offset, LE).unwrap();

                self.requests.alloc(req_idx, req_tag, port, ip);

                // start connect to target
                super::proxy_request(self, tl, req_idx, req_tag, port, ip);
            }
            _ => {
                error!("[Tunnel]unsupport cmd:{:?}, discard msg", cmd);
            }
        }
    }

    pub fn on_dns_reply(&self, data: Vec<u8>, len: usize, port: u16, ip32: u32) {
        let hsize = 6; // port + ipv4
        let buf = &mut vec![0; hsize + len];
        let header = &mut buf[0..hsize];
        let offset = &mut 0;
        header.write_with::<u16>(offset, port, LE).unwrap();
        header.write_with::<u32>(offset, ip32, LE).unwrap();

        let msg_body = &mut buf[hsize..];
        msg_body.copy_from_slice(&data[..len]);

        let wmsg = Message::from(&buf[..]);
        let tx = &self.tx;
        let result = tx.unbounded_send((std::u16::MAX, 0, wmsg));
        match result {
            Err(e) => {
                error!(
                    "[Tunnel]on_dns_reply tun send error:{}, tun_tx maybe closed",
                    e
                );
            }
            _ => info!("[Tunnel]on_dns_reply unbounded_send request msg",),
        }
    }

    fn on_pong(&mut self, msg: Message) {
        let bs = msg.into_data();
        let len = bs.len();
        if len != 8 {
            error!("[Tunnel]pong data length({}) != 8", len);
            return;
        }

        // reset ping count
        self.ping_count = 0;

        let offset = &mut 0;
        let timestamp = bs.read_with::<u64>(offset, LE).unwrap();

        let in_ms = self.get_elapsed_milliseconds();
        assert!(in_ms >= timestamp, "[Tunnel]pong timestamp > now!");

        let rtt = in_ms - timestamp;
        let rtt = rtt as i64;
        self.append_rtt(rtt);
    }

    pub fn on_closed(&mut self) {
        // free all requests
        let reqs = &mut self.requests;
        reqs.clear_all();

        info!(
            "[Tunnel]tunnel live duration {} minutes",
            self.time.elapsed().as_secs() / 60
        );
    }

    fn get_request_tx(&self, req_idx: u16, req_tag: u16) -> Option<UnboundedSender<Bytes>> {
        let requests = &self.requests;
        let req_idx = req_idx as usize;
        if req_idx >= requests.elements.len() {
            return None;
        }

        let req = &requests.elements[req_idx];
        if req.tag == req_tag && req.request_tx.is_some() {
            match req.request_tx {
                None => {
                    return None;
                }
                Some(ref tx) => {
                    return Some(tx.clone());
                }
            }
        }

        None
    }

    pub fn save_request_tx(
        &mut self,
        tx: UnboundedSender<bytes::Bytes>,
        req_idx: u16,
        req_tag: u16,
    ) -> Result<(), ()> {
        let requests = &mut self.requests;
        let req_idx = req_idx as usize;
        if req_idx >= requests.elements.len() {
            return Err(());
        }

        let req = &mut requests.elements[req_idx];
        if req.tag != req_tag {
            return Err(());
        }

        req.request_tx = Some(tx);

        Ok(())
    }

    pub fn save_request_trigger(
        &mut self,
        trigger: Trigger,
        req_idx: u16,
        req_tag: u16,
    ) -> Result<(), ()> {
        let requests = &mut self.requests;
        let req_idx = req_idx as usize;
        if req_idx >= requests.elements.len() {
            return Err(());
        }

        let req = &mut requests.elements[req_idx];
        if req.tag != req_tag {
            return Err(());
        }

        req.trigger = Some(trigger);

        Ok(())
    }

    fn free_request_tx(&mut self, req_idx: u16, req_tag: u16) {
        let requests = &mut self.requests;
        let req_idx = req_idx as usize;
        if req_idx >= requests.elements.len() {
            return;
        }

        let req = &mut requests.elements[req_idx];
        if req.tag == req_tag && req.request_tx.is_some() {
            info!(
                "[Tunnel]free_request_tx, req_idx:{}, req_tag:{}",
                req_idx, req_tag
            );
            req.request_tx = None;
        }
    }

    pub fn on_request_closed(&mut self, req_idx: u16, req_tag: u16) {
        info!("[Tunnel]on_request_closed, req_idx:{}", req_idx);

        if !self.check_req_valid(req_idx, req_tag) {
            return;
        }

        let reqs = &mut self.requests;
        let r = reqs.free(req_idx, req_tag);
        if r {
            info!(
                "[Tunnel]on_request_closed, tun index:{}, sub req_count by 1",
                req_idx
            );
            self.req_count -= 1;

            // send request to agent
            let hsize = THEADER_SIZE;
            let buf = &mut vec![0; hsize];

            let th = THeader::new(Cmd::ReqClientClosed, req_idx, req_tag);
            let msg_header = &mut buf[0..hsize];
            th.write_to(msg_header);

            // websocket message
            let wmsg = Message::from(&buf[..]);

            // send to peer, should always succeed
            if let Err(e) = self.tx.unbounded_send((std::u16::MAX, 0, wmsg)) {
                error!("[Tunnel]send_request_closed_to_server tx send failed:{}", e);
            }
        }
    }

    pub fn on_request_connect_error(&mut self, req_idx: u16, req_tag: u16) {
        info!("[Tunnel]on_request_connect_error, req_idx:{}", req_idx);
        self.on_request_closed(req_idx, req_tag);
    }

    pub fn on_request_msg(&mut self, message: BytesMut, req_idx: u16, req_tag: u16) -> bool {
        info!("[ReqMgr]on_request_msg, req:{}", req_idx);

        if !self.check_req_valid(req_idx, req_tag) {
            return false;
        }

        let size = message.len();
        let hsize = THEADER_SIZE;
        let buf = &mut vec![0; hsize + size];

        let th = THeader::new_data_header(req_idx, req_tag);
        let msg_header = &mut buf[0..hsize];
        th.write_to(msg_header);
        let msg_body = &mut buf[hsize..];
        msg_body.copy_from_slice(message.as_ref());

        let wmsg = Message::from(&buf[..]);
        let result = self.tx.unbounded_send((req_idx, req_tag, wmsg));
        match result {
            Err(e) => {
                error!("[ReqMgr]request tun send error:{}, tun_tx maybe closed", e);
                return false;
            }
            _ => {
                info!("[ReqMgr]unbounded_send request msg, req_idx:{}", req_idx);
                self.flowctl_quota_decrease(req_idx);
            }
        }

        true
    }

    fn flowctl_quota_decrease(&mut self, req_idx: u16) {
        let requests = &mut self.requests;
        let req_idx2 = req_idx as usize;
        let req = &mut requests.elements[req_idx2];

        if req.quota > 0 {
            req.quota -= 1;
        }
    }

    pub fn flowctl_quota_increase(&mut self, req_idx: u16, req_tag: u16) {
        // check req valid
        if !self.check_req_valid(req_idx, req_tag) {
            return;
        }

        let requests = &mut self.requests;
        let req_idx2 = req_idx as usize;
        let req = &mut requests.elements[req_idx2];

        req.quota += 1;

        if req.wait_task.is_some() {
            let wait_task = req.wait_task.take().unwrap();
            wait_task.notify();
        }
    }

    pub fn flowctl_quota_poll(&mut self, req_idx: u16, req_tag: u16) -> bool {
        if !self.check_req_valid(req_idx, req_tag) {
            // just resume the task
            return true;
        }

        let requests = &mut self.requests;
        let req_idx2 = req_idx as usize;
        let req = &mut requests.elements[req_idx2];
        if req.quota < 1 {
            req.wait_task = Some(futures::task::current());
            return false;
        }

        true
    }

    pub fn on_request_recv_finished(&mut self, req_idx: u16, req_tag: u16) {
        info!("[ReqMgr]on_request_recv_finished:{}", req_idx);

        if !self.check_req_valid(req_idx, req_tag) {
            return;
        }

        let hsize = THEADER_SIZE;
        let buf = &mut vec![0; hsize];

        let th = THeader::new(Cmd::ReqClientFinished, req_idx, req_tag);
        let msg_header = &mut buf[0..hsize];
        th.write_to(msg_header);

        let wmsg = Message::from(&buf[..]);
        let result = self.tx.unbounded_send((std::u16::MAX, 0, wmsg));

        match result {
            Err(e) => {
                error!(
                    "[ReqMgr]on_request_recv_finished, tun send error:{}, tun_tx maybe closed",
                    e
                );
            }
            _ => {}
        }
    }

    fn check_req_valid(&self, req_idx: u16, req_tag: u16) -> bool {
        let requests = &self.requests;
        let req_idx2 = req_idx as usize;
        if req_idx2 >= requests.elements.len() {
            return false;
        }

        let req = &requests.elements[req_idx2];
        if !req.is_inused {
            return false;
        }

        if req.tag != req_tag {
            return false;
        }

        true
    }

    pub fn close_rawfd(&self) {
        info!("[Tunnel]close_rawfd, idx:{}", self.tunnel_id);
        let r = shutdown(self.rawfd, Shutdown::Both);
        match r {
            Err(e) => {
                info!("[Tunnel]close_rawfd failed:{}", e);
            }
            _ => {}
        }
    }

    fn append_rtt(&mut self, rtt: i64) {
        let rtt_remove = self.rtt_queue[self.rtt_index];
        self.rtt_queue[self.rtt_index] = rtt;
        let len = self.rtt_queue.len();
        self.rtt_index = (self.rtt_index + 1) % len;

        self.rtt_sum = self.rtt_sum + rtt - rtt_remove;
    }

    fn get_elapsed_milliseconds(&self) -> u64 {
        let in_ms = self.time.elapsed().as_millis();
        in_ms as u64
    }

    pub fn send_ping(&mut self) -> bool {
        let ping_count = self.ping_count;
        if ping_count > 10 {
            return true;
        }

        let timestamp = self.get_elapsed_milliseconds();
        let mut bs1 = vec![0 as u8; 8];
        let bs = &mut bs1[..];
        let offset = &mut 0;
        bs.write_with::<u64>(offset, timestamp, LE).unwrap();

        let wmsg = Message::Ping(bs1);
        let r = self.tx.unbounded_send((std::u16::MAX, 0, wmsg));
        match r {
            Err(e) => {
                error!("[Tunnel]tunnel send_ping error:{}", e);
            }
            _ => {
                self.ping_count += 1;
                if ping_count > 0 {
                    // TODO: fix accurate RTT?
                    self.append_rtt((ping_count as i64) * (KEEP_ALIVE_INTERVAL as i64));
                }
            }
        }

        true
    }
}
