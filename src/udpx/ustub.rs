
use tokio::sync::mpsc::UnboundedSender;
use std::net::SocketAddr;
use std::io::Error;
use std::result::Result;
use log::{error, info};
use futures::prelude::*;
use stream_cancel::{Trigger, Tripwire};
use std::os::unix::io::RawFd;
use nix::sys::socket::{shutdown, Shutdown};
use tokio::net::UdpSocket;
use bytes::Bytes;
use crate::lws::{WMessage};
use std::os::unix::io::AsRawFd;
use fnv::FnvHashSet as HashSet;
use std::cell::RefCell;
use std::rc::Rc;

type TxType = UnboundedSender<(bytes::Bytes, std::net::SocketAddr)>;
type TargetSet = Rc<RefCell<HashSet<SocketAddr>>>;

pub struct UStub {
    rawfd: RawFd,
    tx: Option<TxType>,
    tigger: Option<Trigger>,
    target_set: TargetSet,
    src_addr: SocketAddr,
}

impl UStub {
    pub fn new(src_addr: &SocketAddr, tunnel_tx: UnboundedSender<WMessage>, ll: super::LongLiveX) -> Result<Self, Error> {
        let mut stub = UStub {
            rawfd: 0,
            tx: None, 
            tigger: None,
            target_set: Rc::new(RefCell::new(HashSet::default())),
            src_addr: *src_addr,
        };

        stub.start_udp_socket(src_addr, tunnel_tx, ll)?;

        Ok(stub)
    }

    pub fn on_udp_proxy_north(&mut self, msg: Bytes, dst_addr: SocketAddr) {
        if self.tx.is_none() {
            error!("[UStub] on_udp_proxy_north failed, no tx");
            return;
        }

        info!("[UStub]on_udp_proxy_north, src_addr:{} dst_addr:{}, len:{}", self.src_addr, dst_addr, msg.len());
        self.target_set.borrow_mut().insert(dst_addr);
        match self.tx.as_ref().unwrap().send((msg, dst_addr)){
            Err(e) => {
                error!("[UStub]on_udp_proxy_north, send tx msg failed:{}", e);
            }
            _ => {}
        }
    }

    pub fn cleanup(&mut self) {
        // TODO: close socket and send fut
        self.close_rawfd();

        self.tx = None;
        self.tigger = None;
    }

    fn close_rawfd(&self) {
        info!("[UStub]close_rawfd");
        let r = shutdown(self.rawfd, Shutdown::Both);
        match r {
            Err(e) => {
                info!("[UStub]close_rawfd failed:{}", e);
            }
            _ => {}
        }
    }

    fn set_tx(&mut self, tx2: TxType, trigger: Trigger) {
        self.tx = Some(tx2); 
        self.tigger = Some(trigger);
    }

    fn start_udp_socket(&mut self, src_addr: &SocketAddr,
        tunnel_tx: UnboundedSender<WMessage>, ll: super::LongLiveX) -> std::result::Result<(), Error> {
        // let sockudp = std::net::UdpSocket::new();
        let local_addr: SocketAddr = "0.0.0.0:0".parse().unwrap();
        let socket_udp = std::net::UdpSocket::bind(local_addr)?;
        let rawfd = socket_udp.as_raw_fd();
        let a = UdpSocket::from_std(socket_udp)?;
        
        let udp_framed = tokio_util::udp::UdpFramed::new(a, tokio_util::codec::BytesCodec::new());
        let (a_sink, a_stream) = udp_framed.split();

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let (trigger, tripwire) = Tripwire::new();

        self.set_tx(tx, trigger);
        self.rawfd = rawfd;

        let ll2 = ll.clone();
        let src_addr1 = *src_addr;
        let src_addr2 = *src_addr;
        let target_hashset = self.target_set.clone();

        // send future
        let send_fut = rx.map(move |x|{Ok(x)}).forward(a_sink);
        let receive_fut = async move {
            let mut a_stream = a_stream.take_until(tripwire);
            while let Some(rr) = a_stream.next().await {
                match rr {
                    Ok((message, north_src_addr)) => {
                        // ONLY those north ip in target set cand send packets to device
                        if target_hashset.borrow().contains(&north_src_addr) {
                            info!("[UStub]start_udp_socket recv, src_addr:{} dst_addr:{}, len:{}", north_src_addr, src_addr1, message.len());
                            let rf = ll.borrow();
                            // post to manager
                            rf.on_udp_msg_south(message, &north_src_addr, &src_addr1, tunnel_tx.clone());
                        } else {
                            error!("[UStub]start_udp_socket recv, src_addr:{} dst_addr:{}, len:{}, not in target set", north_src_addr, src_addr1, message.len());
                            break;
                        }
                    },
                    Err(e) => { error!("[UStub] start_udp_socket a_stream.next failed:{}", e); break;}
                };
            };
        };

        // Wait for one future to complete.
        let select_fut = async move {
            future::select(receive_fut.boxed_local(), send_fut).await;
            info!("[UStub] udp both future completed");
            let mut rf = ll2.borrow_mut();
            rf.on_ustub_closed(&src_addr2);
        };

        tokio::task::spawn_local(select_fut);

        Ok(())
    }
}
