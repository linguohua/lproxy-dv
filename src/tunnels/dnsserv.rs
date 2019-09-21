use super::{LongLiveTun, Tunnel};
use futures::try_ready;
use log::error;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::prelude::*;
use tokio::runtime::current_thread;
use tokio::timer::Delay;

pub fn proxy_dns(t: &Tunnel, tl: LongLiveTun, msg_buf: Vec<u8>, port: u16, ip32: u32) {
    let addr = t.dns_server_addr.as_ref().unwrap();
    let local_addr: SocketAddr = "0.0.0.0:0".parse().unwrap();
    let udp = UdpSocket::bind(&local_addr).unwrap();

    const MAX_DATAGRAM_SIZE: usize = 600;
    let fut = udp
        .send_dgram(msg_buf, addr)
        .and_then(move |(socket, _)| {
            // timeout with 2 seconds
            let timeout = 2000;
            let rd = RecvDgram::new(socket, vec![0u8; MAX_DATAGRAM_SIZE], timeout);
            let recv_fut = rd.and_then(move |(_, data, len, _)| {
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

struct RecvDgram<T> {
    state: Option<RecvDgramInner<T>>,
    delay: Delay,
}

struct RecvDgramInner<T> {
    /// The socket
    pub socket: UdpSocket,
    /// The buffer
    pub buffer: T,
}

impl<T> RecvDgram<T> {
    pub(crate) fn new(socket: UdpSocket, buffer: T, milliseconds: u64) -> RecvDgram<T> {
        let inner = RecvDgramInner {
            socket: socket,
            buffer: buffer,
        };
        let when = Instant::now() + Duration::from_millis(milliseconds);
        let delay = Delay::new(when);
        RecvDgram {
            state: Some(inner),
            delay: delay,
        }
    }
}

impl<T> Future for RecvDgram<T>
where
    T: AsMut<[u8]>,
{
    type Item = (UdpSocket, T, usize, SocketAddr);
    type Error = std::io::Error;

    fn poll(&mut self) -> Poll<Self::Item, std::io::Error> {
        match self.delay.poll() {
            Ok(Async::Ready(_)) => {
                return Err(std::io::Error::from(std::io::ErrorKind::TimedOut));
            }
            _ => {}
        }

        let (n, addr) = {
            let ref mut inner = self
                .state
                .as_mut()
                .expect("RecvDgram polled after completion");

            try_ready!(inner.socket.poll_recv_from(inner.buffer.as_mut()))
        };

        let inner = self.state.take().unwrap();
        Ok(Async::Ready((inner.socket, inner.buffer, n, addr)))
    }
}
