use crate::config::{EtcdConfig, ServerCfg};
use crate::lws::{self, LwsFramed};
use crate::service::SubServiceCtlCmd;
use crate::service::TunMgrStub;
use crate::tunnels::find_str_from_query_string;
use failure::Error;
use log::{error, info};
use native_tls::Identity;
use std::cell::RefCell;
use std::fs::File;
use std::hash::{Hash, Hasher};
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

type WSStream = LwsFramed<TlsStream<TcpStream>>;
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
    dns_tun_path: String,
    token_key: String,
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
            dns_tun_path: etcdcfg.dns_tun_path.to_string(),
        }))
    }

    pub fn init(&mut self, s: LongLive) -> Result<(), Error> {
        self.start_server(s)
    }

    pub fn stop(&mut self) {
        self.listener_trigger = None;
    }

    pub fn update_etcd_cfg(&mut self, etcdcfg: &EtcdConfig) {
        info!("[Server] listener update update_etcd_cfg");

        self.tun_path = etcdcfg.tun_path.to_string();
        self.dns_tun_path = etcdcfg.dns_tun_path.to_string();
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
                    .map_err(|tslerr| {
                        error!("TLS Accept error:{}", tslerr);
                        std::io::Error::from(std::io::ErrorKind::NotConnected)
                    })
                    .and_then(move |tls| {
                        // handshake
                        let handshake = lws::do_server_hanshake(tls);
                        let handshake = handshake.and_then(move |(lsocket, path)| {
                            if path.is_some() {
                                let p = path.unwrap();
                                info!("[Server]path:{}", p);
                                let lstream = lws::LwsFramed::new(lsocket, None);
                                let s = ll.clone();
                                let mut s = s.borrow_mut();
                                if p.contains(&s.dns_tun_path) {
                                    s.on_accept_proxy_websocket(rawfd, lstream, true, p);
                                } else if p.contains(&s.tun_path) {
                                    s.on_accept_proxy_websocket(rawfd, lstream, false, p);
                                }
                            }

                            Ok(())
                        });

                        handshake
                    })
                    .map_err(|err| {
                        error!("[Server]TLS accept error: {:?}", err);
                        ()
                    });

                current_thread::spawn(tls_accept);

                Ok(())
            })
            .map_err(|err| {
                error!("[Server]server error {:?}", err);
            });

        current_thread::spawn(server);

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
                if let Err(e) = tx.ctl_tx.unbounded_send(cmd) {
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
