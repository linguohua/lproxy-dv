use super::{ServerCfg, SERVICE_MONITOR_INTERVAL};
use etcd::kv::{self, KeyValueInfo, WatchError, WatchOptions};
use etcd::Error;
use etcd::{Client, Response};
use futures::Future;
use std::fmt;

const ETCD_CFG_ROOT: &str = "/dv/global/";
const ETCD_CFG_TUN_PATH: &str = "/dv/global/tun_path";
const ETCD_CFG_DNS_TUN_PATH: &str = "/dv/global/dns_tun_path";
const ETCD_CFG_HUB_GRPC_ADDR: &str = "/dv/global/hub_grpc_addr";
const ETCD_CFG_DV_INSTANCE_ROOT: &str = "/dv/instances";

pub struct EtcdConfig {
    pub tun_path: String,
    pub dns_tun_path: String,
    pub hub_grpc_addr: String,
}

impl fmt::Display for EtcdConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(tun_path:{}, dns_tun_path:{}, grpc_addr:{})",
            self.tun_path, self.dns_tun_path, self.hub_grpc_addr
        )
    }
}

impl EtcdConfig {
    pub fn load_from_etcd(
        server_cfg: &ServerCfg,
    ) -> impl Future<Item = EtcdConfig, Error = Vec<Error>> {
        println!(
            "load_from_etcd, addr:{}, user:{}, passw:{}",
            &server_cfg.etcd_addr, &server_cfg.etcd_user, &server_cfg.etcd_password
        );

        // Create a client to access a single cluster member. Addresses of multiple cluster
        // members can be provided and the client will try each one in sequence until it
        // receives a successful response.
        let auth = if server_cfg.etcd_user.len() > 0 {
            Some(etcd::BasicAuth {
                username: server_cfg.etcd_user.to_string(),
                password: server_cfg.etcd_password.to_string(),
            })
        } else {
            None
        };

        let client = Client::https(&[&server_cfg.etcd_addr], auth).unwrap();

        // Once the key has been set, ask for details about it.
        let get_request = kv::get(&client, ETCD_CFG_TUN_PATH, kv::GetOptions::default());
        let get_request = get_request.and_then(move |response| {
            let tun_path = response.data.node.value.unwrap();

            //println!("etcd get tun path ok:{}", tun_path);

            let get_request2 = kv::get(&client, ETCD_CFG_DNS_TUN_PATH, kv::GetOptions::default());
            get_request2.and_then(move |response2| {
                let tun_path2 = response2.data.node.value.unwrap();
                //println!("etcd get dns tun path ok:{}", tun_path2);

                let get_request3 =
                    kv::get(&client, ETCD_CFG_HUB_GRPC_ADDR, kv::GetOptions::default());
                get_request3.and_then(move |response3| {
                    let grpc_addr = response3.data.node.value.unwrap();
                    //println!("etcd get grpc address ok:{}", grpc_addr);

                    let ecfg = EtcdConfig {
                        tun_path,
                        dns_tun_path: tun_path2,
                        hub_grpc_addr: grpc_addr,
                    };

                    Ok(ecfg)
                })
            })
        });

        get_request
    }

    pub fn monitor_etcd_cfg(
        server_cfg: &ServerCfg,
    ) -> impl Future<Item = Response<KeyValueInfo>, Error = WatchError> {
        let auth = if server_cfg.etcd_user.len() > 0 {
            Some(etcd::BasicAuth {
                username: server_cfg.etcd_user.to_string(),
                password: server_cfg.etcd_password.to_string(),
            })
        } else {
            None
        };

        let client = Client::https(&[&server_cfg.etcd_addr], auth).unwrap();

        let wo = WatchOptions {
            index: None,
            recursive: true,
            timeout: None, // without timeout, the future maybe hang forever
        };

        kv::watch(&client, ETCD_CFG_ROOT, wo).and_then(|response| Ok(response))
    }
}

pub fn etcd_write_instance_data(
    server_cfg: &ServerCfg,
) -> impl Future<Item = (), Error = Vec<Error>> {
    let auth = if server_cfg.etcd_user.len() > 0 {
        Some(etcd::BasicAuth {
            username: server_cfg.etcd_user.to_string(),
            password: server_cfg.etcd_password.to_string(),
        })
    } else {
        None
    };
    let client = Client::https(&[&server_cfg.etcd_addr], auth).unwrap();

    let uuid = &server_cfg.uuid;
    let dir = format!("{}/{}", ETCD_CFG_DV_INSTANCE_ROOT, uuid);
    let key_address = format!("{}/address", dir);
    let key_value = server_cfg.external_addr.to_string();
    let key_address2 = format!("{}/grpc_address", dir);
    let key_value2 = format!("{}:{}", server_cfg.grpc_addr, server_cfg.grpc_port);

    kv::set(&client, &key_address, &key_value, None).and_then(move |_| {
        kv::set(&client, &key_address2, &key_value2, None).and_then(move |_| {
            kv::update_dir(&client, &dir, Some(SERVICE_MONITOR_INTERVAL * 2))
                .and_then(move |_| Ok(()))
        })
    })
}

pub fn etcd_update_instance_ttl(
    server_cfg: &ServerCfg,
) -> impl Future<Item = (), Error = Vec<Error>> {
    let auth = if server_cfg.etcd_user.len() > 0 {
        Some(etcd::BasicAuth {
            username: server_cfg.etcd_user.to_string(),
            password: server_cfg.etcd_password.to_string(),
        })
    } else {
        None
    };

    let client = Client::https(&[&server_cfg.etcd_addr], auth).unwrap();

    let uuid = &server_cfg.uuid;
    let dir = format!("{}/{}", ETCD_CFG_DV_INSTANCE_ROOT, uuid);
    kv::update_dir(&client, &dir, Some(SERVICE_MONITOR_INTERVAL * 2)).and_then(|_| Ok(()))
}
