use super::{LongLiveTun, Tunnel};
use futures::ready;
use futures::prelude::*;
use log::error;
use std::net::SocketAddr;
use std::time::{Duration};
use tokio::net::UdpSocket;
use tokio::time::Delay;
use std::pin::Pin;
use futures::task::{Context, Poll};
use std::io::Error;

pub fn proxy_dns(_: &Tunnel, tl: LongLiveTun, msg_buf: Vec<u8>, port: u16, ip32: u32) {
    
    let local_addr: SocketAddr = "0.0.0.0:0".parse().unwrap();
    let socket_udp = std::net::UdpSocket::bind(local_addr).unwrap();
    let mut udp = UdpSocket::from_std(socket_udp).unwrap();

    // FOR DNS msg, 600 should be enough
    const MAX_DATAGRAM_SIZE: usize = 600;
    let fut = async move {
        let tl2 = tl.clone();
        let addr = tl.borrow().dns_server_addr.as_ref().unwrap().to_string();
        match udp.send_to(&msg_buf, addr).await {
            Ok(_) => {
                let timeout = 2000;
                match RecvDgram::new(udp, vec![0u8; MAX_DATAGRAM_SIZE], timeout).await {
                    Ok((_, data, len, _)) => {
                        tl2.borrow().on_dns_reply(data, len, port, ip32);
                    }
                    Err(e) => {
                        error!("[DnsProxy]query dns failed:{}", e);
                    }
                }
            }
            Err(e) => {
                error!("[DnsProxy]query dns failed:{}", e);
            }
        }
    };

    tokio::task::spawn_local(fut);
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
        let delay = tokio::time::delay_for(Duration::from_millis(milliseconds));
        RecvDgram {
            state: Some(inner),
            delay: delay,
        }
    }
}

impl<T> Future for RecvDgram<T>
where
    T: AsMut<[u8]>+Unpin,
{
    type Output = std::result::Result<(UdpSocket, T, usize, SocketAddr), Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>{
        let self_mut = self.get_mut();
        if self_mut.delay.is_elapsed() {
            return Poll::Ready(Err(std::io::Error::from(std::io::ErrorKind::TimedOut)));
        }

        let result = {
            let ref mut inner = self_mut
                .state
                .as_mut()
                .expect("RecvDgram polled after completion");

            ready!(inner.socket.poll_recv_from(cx, inner.buffer.as_mut()))
        };

        match result {
            Ok((n, addr)) => {
                let inner = self_mut.state.take().unwrap();
                return Poll::Ready(Ok((inner.socket, inner.buffer, n, addr)));
            }
            Err(e) => {
                return Poll::Ready(Err(e));
            }
        }

    }
}
