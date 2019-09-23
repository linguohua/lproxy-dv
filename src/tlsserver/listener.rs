use crate::config::{TunCfg, DNS_PATH, TUN_PATH};
use crate::service::SubServiceCtlCmd;
use crate::service::TunMgrStub;
use failure::Error;
use log::{error, info};
use native_tls::Identity;
use std::cell::RefCell;
use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::os::unix::io::RawFd;
use std::rc::Rc;
use std::result::Result;
use stream_cancel::{StreamExt, Trigger, Tripwire};
use tokio::prelude::*;
use tokio::runtime::current_thread::{self};
use tokio_tcp::TcpListener;
use tokio_tcp::TcpStream;
use tokio_tls::TlsStream;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::WebSocketStream;
use tungstenite::handshake::server::Request;

type WSStream = WebSocketStream<TlsStream<TcpStream>>;
type LongLive = Rc<RefCell<Listener>>;

pub struct WSStreamInfo {
    pub ws: Option<WSStream>,
    pub path: String,
    pub rawfd: RawFd,
}

pub struct Listener {
    tmstub: Vec<TunMgrStub>,
    dns_tmstub: Vec<TunMgrStub>,
    tmindex: usize,
    dns_tmindex: usize,
    listener_trigger: Option<Trigger>,
    listen_addr: String,
    pkcs12: String,
    pkcs12_password: String,
}

impl Listener {
    pub fn new(cfg: &TunCfg, dns_tmstub: Vec<TunMgrStub>, tmstub: Vec<TunMgrStub>) -> LongLive {
        Rc::new(RefCell::new(Listener {
            dns_tmstub,
            tmstub,
            tmindex: 0,
            dns_tmindex: 0,
            listener_trigger: None,
            listen_addr: cfg.listen_addr.to_string(),
            pkcs12: cfg.pkcs12.to_string(),
            pkcs12_password: cfg.pkcs12_password.to_string(),
        }))
    }

    pub fn init(&mut self, s: LongLive) -> Result<(), Error> {
        self.start_server(s)
    }

    pub fn stop(&mut self) {
        self.listener_trigger = None;
    }

    fn start_server(&mut self, ll: LongLive) -> Result<(), Error> {
        // Bind the server's socket
        let addr = self.listen_addr.parse()?;
        let tcp = TcpListener::bind(&addr)?;
        info!(
            "[Server]listener at:{}, pkcs12:{}",
            self.listen_addr, self.pkcs12
        );

        // Create the TLS acceptor.
        // let der = include_bytes!("identity.p12");
        let mut file = File::open(&self.pkcs12).unwrap();
        let mut identity = vec![];
        file.read_to_end(&mut identity).unwrap();
        let identity = Identity::from_pkcs12(&identity, &self.pkcs12_password).unwrap();
        let tls_acceptor =
            tokio_tls::TlsAcceptor::from(native_tls::TlsAcceptor::builder(identity).build()?);

        let (trigger, tripwire) = Tripwire::new();
        self.listener_trigger = Some(trigger);

        let server = tcp
            .incoming()
            .take_until(tripwire)
            .for_each(move |tcp| {
                let rawfd = tcp.as_raw_fd();
                let ll = ll.clone();
                // tcp.set_nodelay(true).unwrap();

                // Accept the TLS connection.
                let tls_accept = tls_acceptor
                    .accept(tcp)
                    .and_then(move |tls| {
                        let path = Rc::new(RefCell::new(String::new()));
                        let path_clone = path.clone();
                        let cb = move |req: &Request| {
                            // println!("req path:{:?}", req);
                            let mut s = path_clone.borrow_mut();
                            *s = req.path.to_string();

                            Ok(None)
                        };

                        let fut = accept_hdr_async(tls, cb)
                            .and_then(move |ws_stream| {
                                let p = path.borrow();
                                println!("[Server]path:{}", p);

                                let s = ll.clone();
                                let mut s = s.borrow_mut();
                                if p.contains(DNS_PATH) {
                                    s.on_accept_dns_websocket(rawfd, ws_stream, (*p).to_string());
                                } else if p.contains(TUN_PATH) {
                                    s.on_accept_proxy_websocket(rawfd, ws_stream, (*p).to_string());
                                }

                                Ok(())
                            })
                            .or_else(|e| {
                                println!("[Server]websocket error:{}", e);
                                Ok(())
                            });

                        fut
                    })
                    .map_err(|err| {
                        println!("[Server]TLS accept error: {:?}", err);
                        ()
                    });

                current_thread::spawn(tls_accept);

                Ok(())
            })
            .map_err(|err| {
                println!("[Server]server error {:?}", err);
            });

        current_thread::spawn(server);

        Ok(())
    }

    fn on_accept_dns_websocket(&mut self, rawfd: RawFd, ws: WSStream, path: String) {
        let index = self.dns_tmindex;
        if index >= self.dns_tmstub.len() {
            error!("[Server]no tm to handle tcpstream");
            return;
        }

        let wsinfo = WSStreamInfo {
            ws: Some(ws),
            rawfd: rawfd,
            path: path,
        };

        let tx = &self.dns_tmstub[index];
        let cmd = SubServiceCtlCmd::TcpTunnel(wsinfo);
        if let Err(e) = tx.ctl_tx.unbounded_send(cmd) {
            error!("[Server]send req to tm failed:{}", e);
        }

        // move to next tm
        self.dns_tmindex = (index + 1) % self.dns_tmstub.len();
    }

    fn on_accept_proxy_websocket(&mut self, rawfd: RawFd, ws: WSStream, path: String) {
        let index = self.tmindex;
        if index >= self.tmstub.len() {
            error!("[Server]no tm to handle tcpstream");
            return;
        }

        let wsinfo = WSStreamInfo {
            ws: Some(ws),
            rawfd: rawfd,
            path: path,
        };

        let tx = &self.tmstub[index];
        let cmd = SubServiceCtlCmd::TcpTunnel(wsinfo);
        if let Err(e) = tx.ctl_tx.unbounded_send(cmd) {
            error!("[Server]send req to tm failed:{}", e);
        }

        // move to next tm
        self.tmindex = (index + 1) % self.tmstub.len();
    }
}
