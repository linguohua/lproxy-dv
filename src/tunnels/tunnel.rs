use super::{Cmd, LongLiveUD, Reqq, THeader, THEADER_SIZE};
use crate::lws::{RMessage, TMessage, WMessage};
use byte::*;
use futures::task::Waker;
use log::{error, info};
use nix::sys::socket::{shutdown, IpAddr, Shutdown};
use std::cell::RefCell;
use std::net::SocketAddr;
use std::os::unix::io::RawFd;
use std::rc::Rc;
use std::result::Result;
use std::time::Instant;
use stream_cancel::Trigger;
use tokio::sync::mpsc::UnboundedSender;

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

    // rtt_queue: Vec<i64>,
    // rtt_index: usize,
    // rtt_sum: i64,
    req_count: u16,
    pub requests: Reqq,

    pub dns_server_addr: Option<SocketAddr>,

    recv_message_count: usize,
    recv_message_size: usize,

    pub has_flowctl: bool,
    req_quota: u32,

    pub recv_bytes_counter: u64,
    pub send_bytes_counter: u64,

    pub device: LongLiveUD,
}

impl Tunnel {
    pub fn new(
        tid: usize,
        tx: UnboundedSender<WMessage>,
        rawfd: RawFd,
        dns_server_addr: Option<SocketAddr>,
        cap: usize,
        req_quota: u32,
        tun_quota: u32,
        device: LongLiveUD,
        is_for_dns: bool,
    ) -> LongLiveTun {
        let has_flowctl;
        if is_for_dns {
            has_flowctl = false;
            info!(
                "[Tunnel]new dns Tunnel, idx:{}, dns:{:?}",
                tid, dns_server_addr
            );
        } else {
            has_flowctl = tun_quota > 0;
            info!(
                "[Tunnel]new Tunnel, idx:{}, cap:{}, flowctl enable:{}, tun_quota:{}, req_quota:{}",
                tid, cap, has_flowctl, tun_quota, req_quota,
            );
        }

        Rc::new(RefCell::new(Tunnel {
            tunnel_id: tid,
            tx,
            rawfd,
            is_for_dns,
            ping_count: 0,
            time: Instant::now(),

            // rtt_queue: rtt_queue,
            // rtt_index: 0,
            // rtt_sum: 0,
            req_count: 0,

            requests: Reqq::new(cap),
            dns_server_addr: dns_server_addr,
            recv_message_count: 0,
            recv_message_size: 0,

            has_flowctl,
            req_quota,
            recv_bytes_counter: 0,
            send_bytes_counter: 0,
            device,
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
            Cmd::UdpX => {
                self.on_udpx_north(msg);
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
                        self.send_bytes_counter = self.send_bytes_counter + vec.len() as u64;
                        let wmsg = WMessage::new(vec, (3 + THEADER_SIZE) as u16);
                        let result = tx.send(wmsg);
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
                } else if address_type == 0 {
                    // default: Ipv4
                    let a = bs.read_with::<u8>(offset, LE).unwrap(); // 4 bytes
                    let b = bs.read_with::<u8>(offset, LE).unwrap(); // 4 bytes
                    let c = bs.read_with::<u8>(offset, LE).unwrap(); // 4 bytes
                    let d = bs.read_with::<u8>(offset, LE).unwrap(); // 4 bytes
                    let ip = IpAddr::new_v4(a, b, c, d);

                    host = HostInfo::IP(ip);
                } else if address_type == 2 {
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

                self.flowctl_req_quota_increase(req_idx, req_tag, quota_new);
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
        let result = tx.send(wmsg);
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
        let result = tx.send(wmsg);
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
    }

    pub fn on_closed(&mut self) {
        // free all requests
        let reqs = &mut self.requests;
        reqs.clear_all();

        info!(
            "[Tunnel]{} tunnel live duration {} minutes",
            self.tunnel_id,
            self.time.elapsed().as_secs() / 60,
        );
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
        req.request_tx = Some(tx);

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
            if let Err(e) = self.tx.send(wmsg) {
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

        self.recv_bytes_counter = self.recv_bytes_counter + vec.len() as u64;
        // info!(
        //     "[Tunnel]{} send request response to peer, len:{}",
        //     self.tunnel_id,
        //     vec.len()
        // );

        let wmsg = WMessage::new(vec, 0);
        let result = self.tx.send(wmsg);
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
                self.flowctl_req_quota_decrease(req_idx);
            }
        }

        true
    }

    fn flowctl_req_quota_decrease(&mut self, req_idx: u16) {
        let requests = &mut self.requests;
        let req_idx2 = req_idx as usize;
        let req = &mut requests.elements[req_idx2];

        if req.quota > 0 {
            req.quota -= 1;
        }
    }

    pub fn flowctl_req_quota_increase(&mut self, req_idx: u16, req_tag: u16, quota: u16) {
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
            wait_task.wake();
        }
    }

    pub fn flowctl_request_quota_poll(
        &mut self,
        req_idx: u16,
        req_tag: u16,
        wak: Waker,
    ) -> Result<bool, ()> {
        if !self.check_req_valid(req_idx, req_tag) {
            // just resume the task
            info!("[Tunnel]{} flowctl_quota_poll invalid req", self.tunnel_id);
            return Err(());
        }

        let requests = &mut self.requests;
        let req_idx2 = req_idx as usize;
        let req = &mut requests.elements[req_idx2];
        if req.quota < 1 {
            req.wait_task = Some(wak);
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
        let result = self.tx.send(wmsg);

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

    fn get_elapsed_milliseconds(&self) -> u64 {
        let in_ms = self.time.elapsed().as_millis();
        in_ms as u64
    }

    pub fn send_ping(&mut self) -> bool {
        let ping_count = self.ping_count;
        if ping_count > 5 {
            return false;
        }

        let timestamp = self.get_elapsed_milliseconds();
        let mut bs1 = vec![0 as u8; 11]; // 2 bytes length, 1 byte cmd, 8 byte content
        let bs = &mut bs1[..];
        let offset = &mut 0;
        bs.write_with::<u16>(offset, 11, LE).unwrap();
        bs.write_with::<u8>(offset, Cmd::Ping as u8, LE).unwrap();
        bs.write_with::<u64>(offset, timestamp, LE).unwrap();

        let msg = WMessage::new(bs1, 0);
        let r = self.tx.send(msg);
        match r {
            Err(e) => {
                error!("[Tunnel]tunnel send_ping error:{}", e);
            }
            _ => {
                self.ping_count += 1;
            }
        }

        true
    }

    pub fn poll_tunnel_quota_with(&self, bytes_cosume: usize, waker: Waker) -> Result<bool, ()> {
        self.device
            .borrow_mut()
            .poll_tunnel_quota_with(bytes_cosume, waker)
    }

    fn on_udpx_north(&mut self, msg: RMessage) {
        // forward to device
        self.device.borrow_mut().on_udpx_north(self.tx.clone(), msg);
    }
}
