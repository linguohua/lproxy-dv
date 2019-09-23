use super::{HostInfo, LongLiveTun, Tunnel};
use futures::future::Future;
use futures::sync::mpsc::{unbounded, UnboundedReceiver};
use log::{error, info};
use nix::sys::socket::{shutdown, Shutdown};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::unix::io::AsRawFd;
use std::time::Duration;
use stream_cancel::{StreamExt, Tripwire};
use tokio::codec::Decoder;
use tokio::prelude::*;
use tokio::runtime::current_thread;
use tokio_codec::BytesCodec;
use tokio_tcp::TcpStream;

pub fn proxy_request(
    tun: &mut Tunnel,
    tl: LongLiveTun,
    req_idx: u16,
    req_tag: u16,
    port: u16,
    host: HostInfo,
) -> bool {
    let (tx, rx) = unbounded();
    if let Err(_) = tun.save_request_tx(tx, req_idx, req_tag) {
        error!("[Proxy]save_request_tx failed");
        return false;
    }

    match host {
        HostInfo::IP(ip32) => {
            let sockaddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::from(ip32)), port);
            info!("[Proxy] proxy request to ip:{:?}", sockaddr);

            let sockaddr = "127.0.0.1:8001".parse().unwrap();
            let tl0 = tl.clone();
            let fut = TcpStream::connect(&sockaddr)
                .and_then(move |socket| {
                    proxy_request_internal(socket, rx, tl, req_idx, req_tag);

                    Ok(())
                })
                .map_err(move |e| {
                    error!("[Proxy] tcp connect failed:{}", e);
                    let mut tun = tl0.borrow_mut();
                    tun.on_request_connect_error(req_idx, req_tag);
                    ()
                });

            current_thread::spawn(fut);
        }
        HostInfo::Domain(domain) => {
            info!("[Proxy] proxy request to domain:{}:{}", domain, port);
            let h = &domain[..];
            let fut = tokio_dns::TcpStream::connect((h, port))
                .and_then(move |socket| {
                    proxy_request_internal(socket, rx, tl, req_idx, req_tag);

                    Ok(())
                })
                .map_err(|e| {
                    error!("[Proxy] tcp connect failed:{}", e);
                    ()
                });

            current_thread::spawn(fut);
        }
    }

    true
}

fn proxy_request_internal(
    socket: TcpStream,
    rx: UnboundedReceiver<bytes::Bytes>,
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
    let framed = BytesCodec::new().framed(socket);
    let (sink, stream) = framed.split();
    let (trigger, tripwire) = Tripwire::new();

    {
        if let Err(_) = tl
            .borrow_mut()
            .save_request_trigger(trigger, req_idx, req_tag)
        {
            // maybe request has been free
            return;
        }
    }

    let tl2 = tl.clone();
    let tl3 = tl.clone();
    let tl4 = tl.clone();
    let tl5 = tl.clone();;

    // send future
    let send_fut = sink.send_all(rx.map_err(|e| {
        error!("[Proxy]sink send_all failed:{:?}", e);
        std::io::Error::from(std::io::ErrorKind::Other)
    }));

    let send_fut = send_fut.and_then(move |_| {
        info!("[Proxy]send_fut end, index:{}", req_idx);
        // shutdown read direction
        if let Err(e) = shutdown(rawfd, Shutdown::Read) {
            error!("[Proxy]shutdown rawfd error:{}", e);
        }

        Ok(())
    });

    let receive_fut = stream.take_until(tripwire).for_each(move |message| {
        let mut tun_b = tl2.borrow_mut();
        // post to manager
        let prev_error;
        if tun_b.on_request_msg(message, req_idx, req_tag) {
            prev_error = false;
        } else {
            prev_error = true;
        }

        FlowCtl::new(tl5.clone(), req_idx, req_tag, prev_error)
    });

    let receive_fut = receive_fut.and_then(move |_| {
        let mut tun_b = tl3.borrow_mut();
        // client(of request) send finished(FIN), indicate that
        // no more data to send
        tun_b.on_request_recv_finished(req_idx, req_tag);

        Ok(())
    });

    // Wait for both futures to complete.
    let receive_fut = receive_fut
        .map_err(|_| ())
        .join(send_fut.map_err(|_| ()))
        .then(move |_| {
            info!("[Proxy] tcp both futures completed");
            let mut tun = tl4.borrow_mut();
            tun.on_request_closed(req_idx, req_tag);

            Ok(())
        });

    current_thread::spawn(receive_fut);
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
    type Item = ();
    type Error = std::io::Error;

    fn poll(&mut self) -> Poll<Self::Item, std::io::Error> {
        if self.prev_error {
            return Err(std::io::Error::from(std::io::ErrorKind::NotConnected));
        }

        let quota_ready = self
            .tl
            .borrow_mut()
            .flowctl_quota_poll(self.req_idx, self.req_tag);
        if quota_ready {
            //info!("[Proxy] quota ready! {}:{}", self.req_idx, self.req_tag);
            return Ok(Async::Ready(()));
        }

        //info!("[Proxy] quota not ready! {}:{}", self.req_idx, self.req_tag);
        Ok(Async::NotReady)
    }
}
