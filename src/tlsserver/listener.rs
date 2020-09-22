use crate::config::{EtcdConfig, ServerCfg};
use crate::lws::{self, LwsFramed};
use crate::service::SubServiceCtlCmd;
use crate::service::TunMgrStub;
use crate::tunnels::find_str_from_query_string;
use failure::Error;
use futures::prelude::*;
use log::{error, info};
use native_tls::Identity;
use std::cell::RefCell;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::os::unix::io::RawFd;
use std::rc::Rc;
use std::result::Result;
use stream_cancel::{Trigger, Tripwire};
use tokio::net::{TcpListener, TcpStream};
use tokio_tls::TlsStream;

pub type LwsTLS = LwsFramed<TlsStream<TcpStream>>;
pub type LwsHTTP = LwsFramed<TcpStream>;
pub enum WSStream {
    TlsWSStream(LwsTLS),
    HTTPWSStream(LwsHTTP),
}

type LongLive = Rc<RefCell<Listener>>;

pub struct WSStreamInfo {
    pub ws: Option<WSStream>,
    pub path: String,
    pub rawfd: RawFd,
    pub uuid: String,
    pub is_dns: bool,
}

pub struct Listener {
    tmstub: Vec<TunMgrStub>,
    listener_trigger: Option<Trigger>,
    listen_addr: String,
    pkcs12: String,
    pkcs12_password: String,
    tun_path: String,
    token_key: String,
    as_tls: bool,
}

impl Listener {
    pub fn new(cfg: &ServerCfg, etcdcfg: &EtcdConfig, tmstub: Vec<TunMgrStub>) -> LongLive {
        Rc::new(RefCell::new(Listener {
            tmstub,
            listener_trigger: None,
            listen_addr: cfg.listen_addr.to_string(),
            pkcs12: cfg.pkcs12.to_string(),
            pkcs12_password: cfg.pkcs12_password.to_string(),
            tun_path: etcdcfg.tun_path.to_string(),
            token_key: cfg.token_key.to_string(),
            as_tls:cfg.as_tls,
        }))
    }

    pub fn init(&mut self, s: LongLive) -> Result<(), Error> {
        if self.as_tls {
            self.start_tls_server(s)
        } else {
            self.start_http_server(s)
        }
    }

    pub fn stop(&mut self) {
        self.listener_trigger = None;
    }

    pub fn update_etcd_cfg(&mut self, etcdcfg: &EtcdConfig) {
        info!("[Server] listener update update_etcd_cfg");

        self.tun_path = etcdcfg.tun_path.to_string();
    }

    fn start_tls_server(&mut self, ll: LongLive) -> Result<(), Error> {
        // Bind the server's socket
        let addr: std::net::SocketAddr = self.listen_addr.parse()?;
        let socket_tcp = std::net::TcpListener::bind(addr)?;
        let mut tcp = TcpListener::from_std(socket_tcp)?;

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

        let tls_acceptor = Rc::new(RefCell::new(tls_acceptor));
        let (trigger, tripwire) = Tripwire::new();
        self.listener_trigger = Some(trigger);

        let server = async move {
            let mut tcp = tcp.incoming().take_until(tripwire);

            while let Some(tcps) = tcp.next().await {
                match tcps {
                    Ok(tcpx) => {
                        let rawfd = tcpx.as_raw_fd();
                        let ll2 = ll.clone();
                        let tls_acceptor = tls_acceptor.clone();

                        // Accept the TLS connection.
                        let tls_accept = async move {
                            match tls_acceptor.borrow().accept(tcpx).await {
                                Ok(tls) => match lws::do_server_hanshake(tls).await {
                                    Ok((lsocket, path)) => {
                                        if path.is_some() {
                                            let p = path.unwrap();
                                            info!("[Server]path:{}", p);
                                            let lstream = lws::LwsFramed::new(lsocket, None);
                                            let s = ll2.clone();
                                            let mut s = s.borrow_mut();
                                            let dns = find_str_from_query_string(&p, "dns=");
                                            if p.contains(&s.tun_path) {
                                                s.on_accept_proxy_websocket(
                                                    rawfd,
                                                    WSStream::TlsWSStream(lstream),
                                                    dns == "1",
                                                    p,
                                                );
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!("[Server]LWS do_server_hanshake error:{}", e);
                                    }
                                },
                                Err(e) => {
                                    error!("[Server]TLS Accept error:{}", e);
                                }
                            }
                        };
                        // task
                        tokio::task::spawn_local(tls_accept);
                    }
                    Err(e) => {
                        error!("[Server]server accept error {:?}", e);
                        match e.raw_os_error() {
                            Some(code) => {
                                if code != 24 {
                                    // error Os { code: 24, kind: Other, message: "Too many open files" }
                                    break;
                                }
                            }
                            None => {
                                break;
                            }
                        }
                    }
                }
            }

            info!("[Server]accept future completed");
        };
        tokio::task::spawn_local(server);

        Ok(())
    }


    fn start_http_server(&mut self, ll: LongLive) -> Result<(), Error> {
        // Bind the server's socket
        let addr: std::net::SocketAddr = self.listen_addr.parse()?;
        let socket_tcp = std::net::TcpListener::bind(addr)?;
        let mut tcp = TcpListener::from_std(socket_tcp)?;

        info!(
            "[Server]listener at:{}, pkcs12:{}",
            self.listen_addr, self.pkcs12
        );

        // Create the http acceptor.
        let (trigger, tripwire) = Tripwire::new();
        self.listener_trigger = Some(trigger);

        let server = async move {
            let mut tcp = tcp.incoming().take_until(tripwire);

            while let Some(tcps) = tcp.next().await {
                match tcps {
                    Ok(tcpx) => {
                        let rawfd = tcpx.as_raw_fd();
                        let ll2 = ll.clone();

                        let fut_accept = async move {
                            match lws::do_server_hanshake(tcpx).await {
                                Ok((lsocket, path)) => {
                                    if path.is_some() {
                                        let p = path.unwrap();
                                        info!("[Server]path:{}", p);
                                        let lstream = lws::LwsFramed::new(lsocket, None);
                                        let s = ll2.clone();
                                        let mut s = s.borrow_mut();
                                        let dns = find_str_from_query_string(&p, "dns=");
                                        if p.contains(&s.tun_path) {
                                            s.on_accept_proxy_websocket(
                                                rawfd,
                                                WSStream::HTTPWSStream(lstream),
                                                dns == "1",
                                                p,
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!("[Server]LWS do_server_hanshake error:{}", e);
                                }
                            }
                        };

                        // task
                        tokio::task::spawn_local(fut_accept);
                    }
                    Err(e) => {
                        error!("[Server]server accept error {:?}", e);
                        match e.raw_os_error() {
                            Some(code) => {
                                if code != 24 {
                                    // error Os { code: 24, kind: Other, message: "Too many open files" }
                                    break;
                                }
                            }
                            None => {
                                break;
                            }
                        }
                    }
                }
            }

            info!("[Server]accept future completed");
        };
        tokio::task::spawn_local(server);

        Ok(())
    }

    fn on_accept_proxy_websocket(
        &mut self,
        rawfd: RawFd,
        ws: WSStream,
        is_dns: bool,
        path: String,
    ) {
        match self.get_uuid_from_path(&path) {
            Ok(uuid) => {
                let mut hasher = fnv::FnvHasher::default();
                uuid.hash(&mut hasher);
                let index = (hasher.finish() as usize) % self.tmstub.len();

                let wsinfo = WSStreamInfo {
                    ws: Some(ws),
                    rawfd: rawfd,
                    path: path,
                    uuid,
                    is_dns,
                };

                let tx = &self.tmstub[index];
                let cmd = SubServiceCtlCmd::TcpTunnel(wsinfo);
                if let Err(e) = tx.ctl_tx.send(cmd) {
                    error!("[Server]send req to tm failed:{}", e);
                }
            }
            Err(e) => {
                error!("[Server]tunnel no uuid provided, will be dropped:{}", e);
            }
        }
    }

    fn get_uuid_from_path(&self, path: &str) -> std::io::Result<String> {
        let token_str = find_str_from_query_string(path, "tok=");
        let uuidof = crate::token::token_decode(&token_str, self.token_key.as_bytes());
        let uuid;
        if uuidof.is_err() {
            info!("[Server] decode token error:{}", uuidof.err().unwrap());
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "no uuid provided",
            ));
        } else {
            uuid = uuidof.unwrap();
        }

        Ok(uuid)
    }
}
