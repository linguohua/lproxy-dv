use super::{Cmd, Reqq, THeader, THEADER_SIZE};
use crate::config::KEEP_ALIVE_INTERVAL;
use crate::lws::{RMessage, TMessage, WMessage};
use byte::*;
use futures::sync::mpsc::UnboundedSender;
use futures::task::Task;
use log::{error, info};
use nix::sys::socket::IpAddr;
use nix::sys::socket::{shutdown, Shutdown};
use std::cell::RefCell;
use std::net::SocketAddr;
use std::os::unix::io::RawFd;
use std::rc::Rc;
use std::result::Result;
use std::time::Instant;
use stream_cancel::Trigger;

pub type LongLiveTun = Rc<RefCell<Tunnel>>;

pub enum HostInfo {
    IP(IpAddr),
    Domain(String),
}

pub struct Tunnel {
    pub tunnel_id: usize,
    pub tx: UnboundedSender<WMessage>,
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

    recv_message_count: usize,
    recv_message_size: usize,

    pub has_flowctl: bool,
    quota_of_interval: usize,
    quota_per_second_in_kbytes: usize,
    pub wait_task: Option<Task>,
    req_quota: u32,
}

impl Tunnel {
    pub fn new(
        tid: usize,
        tx: UnboundedSender<WMessage>,
        rawfd: RawFd,
        dns_server_addr: Option<SocketAddr>,
        cap: usize,
        req_quota: u32,
        quota_per_second_in_kbytes: usize,
    ) -> LongLiveTun {
        info!(
            "[Tunnel]new Tunnel, idx:{}, cap:{}, dns:{:?}, flowctl enable:{}, kbytes per second:{}, quota:{}",
            tid,
            cap,
            dns_server_addr,
            quota_per_second_in_kbytes > 0,
            quota_per_second_in_kbytes,
            req_quota
        );

        let size = 5;
        let rtt_queue = vec![0; size];
        let is_for_dns = dns_server_addr.is_some();

        let has_flowctl;
        let quota_of_interval;
        if quota_per_second_in_kbytes != 0 {
            has_flowctl = true;
            quota_of_interval = quota_per_second_in_kbytes * (KEEP_ALIVE_INTERVAL as usize);
        } else {
            has_flowctl = false;
            quota_of_interval = 0;
        }

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

            requests: Reqq::new(cap),
            dns_server_addr: dns_server_addr,
            recv_message_count: 0,
            recv_message_size: 0,

            has_flowctl,
            quota_of_interval,
            quota_per_second_in_kbytes,
            wait_task: None,
            req_quota,
        }))
    }

    pub fn on_tunnel_msg(&mut self, msg: RMessage, tl: LongLiveTun) {
        // info!("[Tunnel]on_tunnel_msg");
        let bs = msg.buf.as_ref().unwrap();
        let bs = &bs[2..]; // skip the length

        let offset = &mut 0;
        let cmd = bs.read_with::<u8>(offset, LE).unwrap();
        let bs = &bs[1..]; // skip cmd
        let cmd = Cmd::from(cmd);

        match cmd {
            Cmd::Ping => {
                // send to per
                self.reply_ping(msg);
            }
            Cmd::Pong => {
                self.on_pong(bs);
            }
            _ => {
                if self.is_for_dns {
                    self.on_tunnel_dns_msg(msg, tl);
                } else {
                    self.on_tunnel_proxy_msg(cmd, msg, tl);
                }
            }
        }
    }

    fn on_tunnel_dns_msg(&mut self, mut msg: RMessage, tl: LongLiveTun) {
        // do udp query
        let mut vec = msg.buf.take().unwrap();
        let bs = &vec[3..];
        let offset = &mut 0;
        let port = bs.read_with::<u16>(offset, LE).unwrap();
        let ip32 = bs.read_with::<u32>(offset, LE).unwrap();

        super::proxy_dns(self, tl, vec.split_off(9), port, ip32);
    }

    fn on_tunnel_proxy_msg(&mut self, cmd: Cmd, mut msg: RMessage, tl: LongLiveTun) {
        let vec = msg.buf.take().unwrap();
        let bs = &vec[3..];
        let th = THeader::read_from(&bs[..]);
        let bs = &bs[THEADER_SIZE..];

        match cmd {
            Cmd::ReqData => {
                // data
                let req_idx = th.req_idx;
                let req_tag = th.req_tag;
                let tx = self.get_request_tx(req_idx, req_tag);
                match tx {
                    None => {
                        info!(
                            "[Tunnel]{}no request found for: {}:{}",
                            self.tunnel_id, req_idx, req_tag
                        );
                        return;
                    }
                    Some(tx) => {
                        // info!(
                        //     "[Tunnel]{} proxy request msg, {}:{}",
                        //     self.tunnel_id, req_idx, req_tag
                        // );
                        let wmsg = WMessage::new(vec, (3 + THEADER_SIZE) as u16);
                        let result = tx.unbounded_send(wmsg);
                        match result {
                            Err(e) => {
                                info!(
                                    "[Tunnel]{} tunnel msg send to request failed:{}",
                                    self.tunnel_id, e
                                );
                                return;
                            }
                            _ => {}
                        }
                    }
                }
            }
            Cmd::ReqClientFinished => {
                // client finished
                let req_idx = th.req_idx;
                let req_tag = th.req_tag;
                info!(
                    "[Tunnel] {} ReqClientFinished, idx:{}, tag:{}",
                    self.tunnel_id, req_idx, req_tag
                );

                self.free_request_tx(req_idx, req_tag);
            }
            Cmd::ReqClientClosed => {
                // client closed
                let req_idx = th.req_idx;
                let req_tag = th.req_tag;
                info!(
                    "[Tunnel]{} ReqClientClosed, idx:{}, tag:{}",
                    self.tunnel_id, req_idx, req_tag
                );

                let reqs = &mut self.requests;
                let r = reqs.free(req_idx, req_tag);
                if r && self.req_count > 0 {
                    self.req_count -= 1;
                }
            }
            Cmd::ReqCreated => {
                let req_idx = th.req_idx;
                let req_tag = th.req_tag;

                let offset = &mut 0;
                let address_type = bs.read_with::<u8>(offset, LE).unwrap();
                let host;
                if address_type == 1 {
                    // domain name
                    let domain_name_len: usize = bs.read_with::<u8>(offset, LE).unwrap() as usize;
                    let domain_name_bytes = &bs[2..(2 + domain_name_len)];
                    host = HostInfo::Domain(
                        std::str::from_utf8(domain_name_bytes).unwrap().to_string(),
                    );

                    *offset = *offset + domain_name_len;
                } else if address_type == 2 {
                    // default: Ipv4
                    let a = bs.read_with::<u8>(offset, LE).unwrap(); // 4 bytes
                    let b = bs.read_with::<u8>(offset, LE).unwrap(); // 4 bytes
                    let c = bs.read_with::<u8>(offset, LE).unwrap(); // 4 bytes
                    let d = bs.read_with::<u8>(offset, LE).unwrap(); // 4 bytes
                    let ip = IpAddr::new_v4(a, b, c, d);

                    host = HostInfo::IP(ip);
                } else if address_type == 3 {
                    let a = bs.read_with::<u16>(offset, LE).unwrap(); // 16 bytes
                    let b = bs.read_with::<u16>(offset, LE).unwrap(); // 16 bytes
                    let c = bs.read_with::<u16>(offset, LE).unwrap(); // 16 bytes
                    let d = bs.read_with::<u16>(offset, LE).unwrap(); // 16 bytes
                    let e = bs.read_with::<u16>(offset, LE).unwrap(); // 16 bytes
                    let f = bs.read_with::<u16>(offset, LE).unwrap(); // 16 bytes
                    let g = bs.read_with::<u16>(offset, LE).unwrap(); // 16 bytes
                    let h = bs.read_with::<u16>(offset, LE).unwrap(); // 16 bytes

                    let ip = IpAddr::new_v6(a, b, c, d, e, f, g, h);

                    host = HostInfo::IP(ip);
                } else {
                    info!(
                        "[Tunnel]{} tunnel recv request create failed, unknown address type:{}",
                        self.tunnel_id, address_type
                    );
                    return;
                }

                // port, u16
                let port = bs.read_with::<u16>(offset, LE).unwrap();
                self.requests.alloc(req_idx, req_tag, self.req_quota);

                // start connect to target
                if super::proxy_request(self, tl, req_idx, req_tag, port, host) {
                    self.req_count += 1;
                }
            }
            Cmd::ReqClientQuota => {
                // quota report
                let req_idx = th.req_idx;
                let req_tag = th.req_tag;

                let offset = &mut 0;
                let quota_new = bs.read_with::<u16>(offset, LE).unwrap();

                self.flowctl_quota_increase(req_idx, req_tag, quota_new);
            }
            _ => {
                error!(
                    "[Tunnel]{} unsupport cmd:{:?}, discard msg",
                    self.tunnel_id, cmd
                );
            }
        }
    }

    pub fn on_dns_reply(&self, data: Vec<u8>, len: usize, port: u16, ip32: u32) {
        let hsize = 2 + 1 + 6; // length + cmd + port + ipv4
        let total = hsize + len;
        let mut buf = vec![0; total];
        let header = &mut buf[0..hsize];
        let offset = &mut 0;
        header.write_with::<u16>(offset, total as u16, LE).unwrap();
        header
            .write_with::<u8>(offset, Cmd::ReqData as u8, LE)
            .unwrap();
        header.write_with::<u16>(offset, port, LE).unwrap();
        header.write_with::<u32>(offset, ip32, LE).unwrap();

        let msg_body = &mut buf[hsize..];
        msg_body.copy_from_slice(&data[..len]);

        let wmsg = WMessage::new(buf, 0);
        let tx = &self.tx;
        let result = tx.unbounded_send(wmsg);
        match result {
            Err(e) => {
                error!(
                    "[Tunnel]{} on_dns_reply tun send error:{}, tun_tx maybe closed",
                    self.tunnel_id, e
                );
            }
            _ => {
                //info!("[Tunnel]on_dns_reply unbounded_send request msg",)
            }
        }
    }

    fn reply_ping(&mut self, mut msg: RMessage) {
        //info!("[Tunnel] reply_ping");
        let mut vec = msg.buf.take().unwrap();
        let bs = &mut vec[2..];
        let offset = &mut 0;
        bs.write_with::<u8>(offset, Cmd::Pong as u8, LE).unwrap();

        let wmsg = WMessage::new(vec, 0);
        let tx = &self.tx;
        let result = tx.unbounded_send(wmsg);
        match result {
            Err(e) => {
                error!(
                    "[Tunnel]{} reply_ping tun send error:{}, tun_tx maybe closed",
                    self.tunnel_id, e
                );
            }
            _ => {
                //info!("[Tunnel]on_dns_reply unbounded_send request msg",)
            }
        }
    }

    fn on_pong(&mut self, bs: &[u8]) {
        //info!("[Tunnel] on_pong");
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
            "[Tunnel]{} tunnel live duration {} minutes",
            self.tunnel_id,
            self.time.elapsed().as_secs() / 60
        );

        if self.wait_task.is_some() {
            let wait_task = self.wait_task.take().unwrap();
            wait_task.notify();
        }
    }

    fn get_request_tx(&self, req_idx: u16, req_tag: u16) -> Option<UnboundedSender<WMessage>> {
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
        tx: UnboundedSender<WMessage>,
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
                "[Tunnel]{} free_request_tx, req_idx:{}, req_tag:{}",
                self.tunnel_id, req_idx, req_tag
            );
            req.request_tx = None;
        }
    }

    pub fn on_request_closed(&mut self, req_idx: u16, req_tag: u16) {
        info!(
            "[Tunnel]{} on_request_closed, req_idx:{}",
            self.tunnel_id, req_idx
        );

        if !self.check_req_valid(req_idx, req_tag) {
            return;
        }

        let reqs = &mut self.requests;
        let r = reqs.free(req_idx, req_tag);
        if r {
            info!(
                "[Tunnel]{} on_request_closed, tun index:{}, sub req_count by 1",
                self.tunnel_id, req_idx
            );
            self.req_count -= 1;

            // send request to agent
            let hsize = 3 + THEADER_SIZE;
            let mut buf = vec![0; hsize];
            let offset = &mut 0;
            let header = &mut buf[..];
            header.write_with::<u16>(offset, hsize as u16, LE).unwrap();
            header
                .write_with::<u8>(offset, Cmd::ReqServerClosed as u8, LE)
                .unwrap();

            let th = THeader::new(req_idx, req_tag);
            let msg_header = &mut buf[3..];
            th.write_to(msg_header);

            // websocket message
            let wmsg = WMessage::new(buf, 0);

            // send to peer, should always succeed
            if let Err(e) = self.tx.unbounded_send(wmsg) {
                error!(
                    "[Tunnel]{} send_request_closed_to_server tx send failed:{}",
                    self.tunnel_id, e
                );
            }
        }
    }

    pub fn on_request_connect_error(&mut self, req_idx: u16, req_tag: u16) {
        info!(
            "[Tunnel]{} on_request_connect_error, req_idx:{}",
            self.tunnel_id, req_idx
        );
        self.on_request_closed(req_idx, req_tag);
    }

    pub fn on_request_msg(&mut self, mut message: TMessage, req_idx: u16, req_tag: u16) -> bool {
        // info!("[Tunnel]{} on_request_msg, req:{}", self.tunnel_id, req_idx);

        if !self.check_req_valid(req_idx, req_tag) {
            return false;
        }

        let mut vec = message.buf.take().unwrap();
        let ll = vec.len();
        let size = vec.len() - (3 + THEADER_SIZE);
        self.recv_message_count += 1;
        self.recv_message_size += size;
        if self.recv_message_count % 200 == 0 {
            info!(
                "[Tunnel]{} average bytesmut length:{}",
                self.tunnel_id,
                self.recv_message_size / self.recv_message_count
            );
        }

        let bs = &mut vec[..];
        let offset = &mut 0;
        bs.write_with::<u16>(offset, ll as u16, LE).unwrap();
        bs.write_with::<u8>(offset, Cmd::ReqData as u8, LE).unwrap();

        let th = THeader::new(req_idx, req_tag);
        let msg_header = &mut vec[3..];
        th.write_to(msg_header);

        // info!(
        //     "[Tunnel]{} send request response to peer, len:{}",
        //     self.tunnel_id,
        //     vec.len()
        // );

        let wmsg = WMessage::new(vec, 0);
        let result = self.tx.unbounded_send(wmsg);
        match result {
            Err(e) => {
                error!(
                    "[Tunnel]{} request tun send error:{}, tun_tx maybe closed",
                    self.tunnel_id, e
                );
                return false;
            }
            _ => {
                // info!("[Tunnel]unbounded_send request msg, req_idx:{}", req_idx);
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

    pub fn flowctl_quota_increase(&mut self, req_idx: u16, req_tag: u16, quota: u16) {
        // check req valid
        if !self.check_req_valid(req_idx, req_tag) {
            return;
        }

        let requests = &mut self.requests;
        let req_idx2 = req_idx as usize;
        let req = &mut requests.elements[req_idx2];

        req.quota += quota as u32;
        if req.wait_task.is_some() {
            let wait_task = req.wait_task.take().unwrap();
            wait_task.notify();
        }
    }

    pub fn flowctl_request_quota_poll(&mut self, req_idx: u16, req_tag: u16) -> Result<bool, ()> {
        if !self.check_req_valid(req_idx, req_tag) {
            // just resume the task
            info!("[Tunnel]{} flowctl_quota_poll invalid req", self.tunnel_id);
            return Err(());
        }

        let requests = &mut self.requests;
        let req_idx2 = req_idx as usize;
        let req = &mut requests.elements[req_idx2];
        if req.quota < 1 {
            req.wait_task = Some(futures::task::current());
            return Ok(false);
        }

        Ok(true)
    }

    pub fn poll_tunnel_quota_with(&mut self, bytes_cosume: usize) -> Result<bool, ()> {
        if self.quota_of_interval < bytes_cosume {
            self.quota_of_interval = 0;
        } else {
            self.quota_of_interval -= bytes_cosume;
        }

        if self.quota_of_interval == 0 {
            self.wait_task = Some(futures::task::current());
            return Ok(false);
        }

        Ok(true)
    }

    pub fn on_request_recv_finished(&mut self, req_idx: u16, req_tag: u16) {
        info!(
            "[Tunnel]{} on_request_recv_finished:{}",
            self.tunnel_id, req_idx
        );

        if !self.check_req_valid(req_idx, req_tag) {
            return;
        }

        // send request to agent
        let hsize = 3 + THEADER_SIZE;
        let mut buf = vec![0; hsize];
        let offset = &mut 0;
        let header = &mut buf[..];
        header.write_with::<u16>(offset, hsize as u16, LE).unwrap();
        header
            .write_with::<u8>(offset, Cmd::ReqServerFinished as u8, LE)
            .unwrap();

        let th = THeader::new(req_idx, req_tag);
        let msg_header = &mut buf[3..];
        th.write_to(msg_header);

        // websocket message
        let wmsg = WMessage::new(buf, 0);
        let result = self.tx.unbounded_send(wmsg);

        match result {
            Err(e) => {
                error!(
                    "[Tunnel]{} on_request_recv_finished, tun send error:{}, tun_tx maybe closed",
                    self.tunnel_id, e
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
                info!("[Tunnel]{} close_rawfd failed:{}", self.tunnel_id, e);
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

    pub fn reset_quota_interval(&mut self) {
        self.quota_of_interval = self.quota_per_second_in_kbytes * (KEEP_ALIVE_INTERVAL as usize);
        if self.wait_task.is_some() {
            let wait_task = self.wait_task.take().unwrap();
            wait_task.notify();
        }
    }

    pub fn send_ping(&mut self) -> bool {
        let ping_count = self.ping_count;
        if ping_count > 10 {
            return true;
        }

        let timestamp = self.get_elapsed_milliseconds();
        let mut bs1 = vec![0 as u8; 11]; // 2 bytes length, 1 byte cmd, 8 byte content
        let bs = &mut bs1[..];
        let offset = &mut 0;
        bs.write_with::<u16>(offset, 11, LE).unwrap();
        bs.write_with::<u8>(offset, Cmd::Ping as u8, LE).unwrap();
        bs.write_with::<u64>(offset, timestamp, LE).unwrap();

        let msg = WMessage::new(bs1, 0);
        let r = self.tx.unbounded_send(msg);
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
