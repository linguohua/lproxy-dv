use super::{HostInfo, LongLiveTun, Tunnel};
use crate::lws::{TcpFramed, WMessage};
use futures::prelude::*;
use tokio::sync::mpsc::{self, UnboundedReceiver};
use log::{error, info};
use nix::sys::socket::{shutdown, Shutdown};
use std::net::SocketAddr;
use std::os::unix::io::AsRawFd;
use std::time::Duration;
use stream_cancel::{Tripwire};
use tokio::net::TcpStream;
use std::io::Error;
use std::pin::Pin;
use futures::task::{Context, Poll};

pub fn proxy_request(
    tun: &mut Tunnel,
    tl: LongLiveTun,
    req_idx: u16,
    req_tag: u16,
    port: u16,
    host: HostInfo,
) -> bool {
    let (tx, rx) = mpsc::unbounded_channel();
    let (trigger, tripwire) = Tripwire::new();
    if let Err(_) = tun.save_request_tx(tx, trigger, req_idx, req_tag) {
        error!("[Proxy]save_request_tx failed");
        return false;
    }

    match host {
        HostInfo::IP(ip) => {
            let sockaddr = SocketAddr::new(ip.to_std(), port);
            // llet sockaddr = "127.0.0.1:8001".parse().unwrap();
            info!("[Proxy] proxy request to ip:{:?}", sockaddr);

            let tl0 = tl.clone();
            let fut = async move {
                match tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(&sockaddr)).await {
                    Ok(socket2) => {
                        match socket2 {
                            Ok(socket) => {
                                proxy_request_internal(socket, rx, tripwire, tl, req_idx, req_tag);
                            }
                            Err(e) => {
                                error!("[Proxy] tcp connect failed:{}", e);
                                let mut tun = tl0.borrow_mut();
                                tun.on_request_connect_error(req_idx, req_tag);
                            }
                        }
                    },
                    Err(e) => {
                        error!("[Proxy] tcp connect timeout :{}", e);
                        let mut tun = tl0.borrow_mut();
                        tun.on_request_connect_error(req_idx, req_tag);
                    }
                }
            };

            tokio::task::spawn_local(fut);
        }
        HostInfo::Domain(domain) => {
            info!("[Proxy] proxy request to domain:{}:{}", domain, port);
            let tl0 = tl.clone();
            
            let fut = async move {
                let h = &domain[..];
                match tokio::time::timeout(Duration::from_secs(5),TcpStream::connect((h, port))).await {
                    Ok(socket2) => {
                        match socket2 {
                            Ok(socket) => {
                                proxy_request_internal(socket, rx, tripwire, tl, req_idx, req_tag);
                            }
                            Err(e) => {
                                error!("[Proxy] tcp connect failed:{}", e);
                                let mut tun = tl0.borrow_mut();
                                tun.on_request_connect_error(req_idx, req_tag);
                            }
                        }
                        
                    }
                    Err(e) => {
                        error!("[Proxy] tcp connect timeout:{}", e);
                        let mut tun = tl0.borrow_mut();
                        tun.on_request_connect_error(req_idx, req_tag);
                    }
                }
            };

            tokio::task::spawn_local(fut);
        }
    }

    true
}

fn proxy_request_internal(
    socket: TcpStream,
    rx: UnboundedReceiver<WMessage>,
    tripwire: Tripwire,
    tl: LongLiveTun,
    req_idx: u16,
    req_tag: u16,
) {
    // config tcp stream
    socket.set_linger(None).unwrap();
    let kduration = Duration::new(3, 0);
    socket.set_keepalive(Some(kduration)).unwrap();
    // socket.set_nodelay(true).unwrap();

    let rawfd = socket.as_raw_fd();
    let framed = TcpFramed::new(socket);
    let (sink, stream) = framed.split();

    let tl2 = tl.clone();
    let tl3 = tl.clone();
    let tl4 = tl.clone();
    let tl5 = tl.clone();

    // send future
    let send_fut = async move {
        match rx.map(|x|{Ok(x)}).forward(sink).await {
            Err(e) => {
                error!("[Proxy]send_fut failed:{}", e);
            }
            _ => {}
        }
        
        info!("[Proxy]send_fut end, index:{}", req_idx);
        // shutdown read direction
        if let Err(e) = shutdown(rawfd, Shutdown::Read) {
            error!("[Proxy]shutdown rawfd error:{}", e);
        }
    };

    let receive_fut = stream.take_until(tripwire).for_each(move |message| {
        let mut tun_b = tl2.borrow_mut();
        // post to manager
        let prev_error;
        match message {
            Ok(m) => {
                if tun_b.on_request_msg(m, req_idx, req_tag) {
                    prev_error = false;
                } else {
                    prev_error = true;
                }
            }
            _ => {
                prev_error = false;
            }
        }

        FlowCtl::new(tl5.clone(), req_idx, req_tag, prev_error).map(|_|{})
    });

    let receive_fut = async move {
        receive_fut.await;
        let mut tun_b = tl3.borrow_mut();
        // client(of request) send finished(FIN), indicate that
        // no more data to send
        tun_b.on_request_recv_finished(req_idx, req_tag);
    };

    // Wait for both futures to complete.
    let join_fut = async move {
        future::join(send_fut, receive_fut).await;
        info!("[Proxy] tcp both futures completed");
        let mut tun = tl4.borrow_mut();
        tun.on_request_closed(req_idx, req_tag);
    };
    tokio::task::spawn_local(join_fut);
}

struct FlowCtl {
    tl: LongLiveTun,
    req_idx: u16,
    req_tag: u16,
    prev_error: bool,
}

impl FlowCtl {
    pub fn new(tl: LongLiveTun, req_idx: u16, req_tag: u16, prev_error: bool) -> FlowCtl {
        FlowCtl {
            tl,
            req_idx,
            req_tag,
            prev_error,
        }
    }
}

impl Future for FlowCtl {
    type Output = std::result::Result<(), Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>{
        let self_mut = self.get_mut();
        if self_mut.prev_error {
            return Poll::Ready(Err(std::io::Error::from(std::io::ErrorKind::NotConnected)));
        }

        let quota_ready = self_mut
            .tl
            .borrow_mut()
            .flowctl_request_quota_poll(self_mut.req_idx, self_mut.req_tag, cx.waker().clone());
        match quota_ready {
            Err(_) => Poll::Ready(Err(std::io::Error::from(std::io::ErrorKind::NotConnected))),
            Ok(t) => {
                if t {
                    return Poll::Ready(Ok(()));
                } else {
                    return Poll::Pending;
                }
            }
        }
    }
}
