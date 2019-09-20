use super::{LongLiveTun, Tunnel};
use log::error;
use std::net::SocketAddr;
use tokio::net::UdpSocket;
use tokio::prelude::*;
use tokio::runtime::current_thread;

pub fn proxy_dns(t: &Tunnel, tl: LongLiveTun, msg_buf: Vec<u8>, port: u16, ip32: u32) {
    let addr = t.dns_server_addr.as_ref().unwrap();
    let local_addr: SocketAddr = "0.0.0.0:0".parse().unwrap();
    let udp = UdpSocket::bind(&local_addr).unwrap();

    const MAX_DATAGRAM_SIZE: usize = 512;
    let fut = udp
        .send_dgram(msg_buf, addr)
        .and_then(move |(socket, _)| {
            let recv_fut = socket
                .recv_dgram(vec![0u8; MAX_DATAGRAM_SIZE])
                .and_then(move |(_, data, len, _)| {
                    // TODO: add recv timeout
                    tl.borrow().on_dns_reply(data, len, port, ip32);
                    Ok(())
                });

            recv_fut
        })
        .map_err(|e| {
            error!("[DnsProxy]query dns failed:{}", e);
            ()
        });

    current_thread::spawn(fut);
}
