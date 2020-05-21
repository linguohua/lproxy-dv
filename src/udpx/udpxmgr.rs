use tokio::sync::mpsc::UnboundedSender;
use std::net::SocketAddr;
use std::cell::RefCell;
use std::rc::Rc;
use log::{error, info};
use bytes::BytesMut;
use super::{LongLiveC,UStub, Cache};
use crate::lws::{RMessage, WMessage};
use bytes::Buf;
use byte::*;
use crate::tunnels::{THEADER_SIZE,Cmd};
use std::net::{IpAddr::{self, V4, V6}, Ipv6Addr, Ipv4Addr};
use super::AddressPair;

pub type LongLiveX = Rc<RefCell<UdpXMgr>>;


pub struct UdpXMgr {
    cache: LongLiveC,
}

impl UdpXMgr {
    pub fn new() -> LongLiveX {
        info!("[UdpXMgr]new UdpXMgr");
        Rc::new(RefCell::new(UdpXMgr {
            cache:  Cache::new(),
        }))
    }

    // pub fn stop(&mut self) {
    //     self.cache.borrow_mut().cleanup();
    // }

    pub fn on_udp_msg_south(&self, msg: BytesMut, src_addr:&SocketAddr, dst_addr: &SocketAddr, tunnel_tx: UnboundedSender<WMessage>) {
        // format packet, send to tx
                // format udp forward packet
        let mut content_size = msg.len();
        if src_addr.is_ipv4() {
            content_size += 7; // 1 + 2 + 4
        } else {
            content_size += 19; // 1 + 2 + 16
        }
        if dst_addr.is_ipv4() {
            content_size += 7; // 1 + 2 + 4
        } else {
            content_size += 19; // 1 + 2 + 16
        }
        let hsize = 2 + 1 + THEADER_SIZE;
        let total = hsize + content_size;

        let mut buf = vec![0; total];
        let bs = &mut buf[..];
        let offset = &mut 0;
        bs.write_with::<u16>(offset, total as u16, LE).unwrap(); // length
        bs.write_with::<u8>(offset, Cmd::UdpX as u8, LE).unwrap(); // cmd
        *offset = UdpXMgr::write_socketaddr(bs, *offset, src_addr);
        *offset = UdpXMgr::write_socketaddr(bs, *offset, dst_addr);
        let bss = &mut buf[*offset..];
        bss.copy_from_slice(&msg);

        // websocket message
        let wmsg = WMessage::new(buf, 0);

        let r = tunnel_tx.send(wmsg);
        match r {
            Err(e) => {
                error!("[UdpXMgr]on_udp_msg_south send error:{}", e);
            }
            _ => {
            }
        }
    }

    pub fn on_udp_proxy_north(&mut self, lx:LongLiveX, tunnel_tx: UnboundedSender<WMessage>, msg: RMessage) {
        let (addr_pair, msgbody) = UdpXMgr::parse_udp_north_msg(msg);
        let cache: &mut Cache = &mut self.cache.borrow_mut();
        let mut stub = cache.get(&addr_pair);
        if stub.is_none() {
            // build new stub
            self.build_ustub(lx, cache, tunnel_tx, &addr_pair);
            stub = cache.get(&addr_pair);
        }

        match stub {
            Some(stub) => {
                stub.on_udp_proxy_north(msgbody);
            }
            None => {
                error!("[UdpXMgr] on_udp_proxy_north failed, no stub found");
            }
        }
    }

    fn parse_udp_north_msg(mut msg: RMessage) -> (AddressPair, bytes::Bytes) {
        let offset = &mut 0;
        let bsv = msg.buf.take().unwrap();
        let bs = &bsv[3..]; // skip the length and cmd

        let src_addr = UdpXMgr::read_socketaddr(bs, offset);
        let dst_addr = UdpXMgr::read_socketaddr(bs, offset);

        let skip = *offset;
        let mut bb = bytes::Bytes::from(bsv);
        bb.advance(skip as usize);

        let addr_pair = AddressPair {
            src_addr,
            dst_addr,
        };

        (addr_pair, bb)
    }

    fn build_ustub(&self, lx:LongLiveX, c: &mut Cache, tunnel_tx: UnboundedSender<WMessage>, addr_pair: &AddressPair) {
        match UStub::new(addr_pair, tunnel_tx, lx) {
            Ok(ustub) => {
                let addr_pair2: AddressPair = *addr_pair;
                c.insert(self.cache.clone(), addr_pair2, ustub);
            }
            Err(e) => {
                error!("[UdpXMgr] build_ustub failed:{}", e);
            }
        }
    }

    pub fn on_ustub_closed(&mut self, addr_pair: &AddressPair) {
        self.cache.borrow_mut().remove(addr_pair);
    }

    fn read_socketaddr(bs: &[u8], offset: &mut usize) -> SocketAddr {
        let port = bs.read_with::<u16>(offset, LE).unwrap();
        let addr_type = bs.read_with::<u8>(offset, LE).unwrap();
        let ipbytes = if addr_type == 0 {
            // ipv4
            let a = bs.read_with::<u8>(offset, LE).unwrap(); // 4 bytes
            let b = bs.read_with::<u8>(offset, LE).unwrap(); // 4 bytes
            let c = bs.read_with::<u8>(offset, LE).unwrap(); // 4 bytes
            let d = bs.read_with::<u8>(offset, LE).unwrap(); // 4 bytes
            let ip = Ipv4Addr::new(a, b, c, d);
            IpAddr::V4(ip)
        } else {
            // ipv6
            let a = bs.read_with::<u16>(offset, LE).unwrap(); // 16 bytes
            let b = bs.read_with::<u16>(offset, LE).unwrap(); // 16 bytes
            let c = bs.read_with::<u16>(offset, LE).unwrap(); // 16 bytes
            let d = bs.read_with::<u16>(offset, LE).unwrap(); // 16 bytes
            let e = bs.read_with::<u16>(offset, LE).unwrap(); // 16 bytes
            let f = bs.read_with::<u16>(offset, LE).unwrap(); // 16 bytes
            let g = bs.read_with::<u16>(offset, LE).unwrap(); // 16 bytes
            let h = bs.read_with::<u16>(offset, LE).unwrap(); // 16 bytes

            let ip = Ipv6Addr::new(a, b, c, d, e, f, g, h);
            IpAddr::V6(ip)
        };

        SocketAddr::from((ipbytes, port))
    }

    fn write_socketaddr(bs: &mut [u8], offset: usize, addr : &SocketAddr) -> usize {
        let mut new_offset = offset;
        let new_offset = &mut new_offset;
        bs.write_with::<u16>(new_offset, addr.port() as u16, LE).unwrap(); // port

        match addr.ip() {
            V4(v4) => {
                bs.write_with::<u8>(new_offset, 0 as u8, LE).unwrap(); // 0, ipv4
                let ipbytes = v4.octets();
                for b in ipbytes.iter() {
                    bs.write_with::<u8>(new_offset, *b, LE).unwrap(); // ip
                }
            }
            V6(v6) => {
                bs.write_with::<u8>(new_offset, 1 as u8, LE).unwrap(); // 0, ipv6
                let ipbytes = v6.segments();
                for b in ipbytes.iter() {
                    bs.write_with::<u16>(new_offset, *b, LE).unwrap(); // ip
                }
            }
        }

        *new_offset
    }
}
