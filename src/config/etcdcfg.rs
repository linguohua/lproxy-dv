use super::{ServerCfg, SERVICE_MONITOR_INTERVAL};
use etcd::kv::{self, KeyValueInfo, WatchError, WatchOptions};
use etcd::Error;
use etcd::{Client, Response};
use std::fmt;
use futures::compat::Compat01As03;

const ETCD_CFG_ROOT: &str = "/dv/global/";
const ETCD_CFG_TUN_PATH: &str = "/dv/global/tun_path";
const ETCD_CFG_HUB_GRPC_ADDR: &str = "/dv/global/hub_grpc_addr";
const ETCD_CFG_DV_INSTANCE_ROOT: &str = "/dv/instances";

pub struct EtcdConfig {
    pub tun_path: String,
    pub hub_grpc_addr: String,
}

impl fmt::Display for EtcdConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(tun_path:{}, grpc_addr:{})",
            self.tun_path, self.hub_grpc_addr
        )
    }
}

impl EtcdConfig {
    pub async fn load_from_etcd(
        server_cfg: &ServerCfg,
    ) -> std::result::Result<EtcdConfig, Vec<Error>> {
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
        let response = Compat01As03::new(get_request).await?;
        let tun_path = response.data.node.value.unwrap();
        let get_request2 = kv::get(&client, ETCD_CFG_HUB_GRPC_ADDR, kv::GetOptions::default());
        let response3 = Compat01As03::new(get_request2).await?;
        let grpc_addr = response3.data.node.value.unwrap();
        //println!("etcd get grpc address ok:{}", grpc_addr);

        let ecfg = EtcdConfig {
            tun_path,
            hub_grpc_addr: grpc_addr,
        };

        Ok(ecfg)
    }

    pub async fn monitor_etcd_cfg(
        server_cfg: &ServerCfg,
    ) -> std::result::Result<Response<KeyValueInfo>, WatchError> {
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

        let rsp = Compat01As03::new(kv::watch(&client, ETCD_CFG_ROOT, wo)).await?;
        Ok(rsp)
    }
}

pub async fn etcd_write_instance_data(
    server_cfg: &ServerCfg,
) -> std::result::Result<(), Vec<Error>> {
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
    let key_value2 = format!("{}:{}", server_cfg.my_grpc_addr, server_cfg.my_grpc_port);

    Compat01As03::new(kv::set(&client, &key_address, &key_value, None)).await?;
    Compat01As03::new(kv::set(&client, &key_address2, &key_value2, None)).await?;
    Compat01As03::new(kv::update_dir(&client, &dir, Some(SERVICE_MONITOR_INTERVAL * 2))).await?;

    Ok(())
}

pub async fn etcd_update_instance_ttl(
    server_cfg: &ServerCfg,
) -> std::result::Result<(), Vec<Error>> {
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
    Compat01As03::new(kv::update_dir(&client, &dir, Some(SERVICE_MONITOR_INTERVAL * 2))).await?;

    Ok(())
}
