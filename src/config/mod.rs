mod logsimple;
pub use logsimple::*;
mod etcdcfg;
pub use etcdcfg::*;

use std::fs::File;
use std::io::{Error, ErrorKind, Read, Result};

pub const QUOTA_RESET_INTERVAL: u64 = 2 * 1000;
pub const KEEPALIVE_INTERVAL: u64 = 30 * 1000;
pub const SERVICE_MONITOR_INTERVAL: u64 = 30 * 1000;

// pub const TUN_PATH: &str = "/tunxKMZ2oFFxixut7cO4nI1Q8pNqNHjGfvobrFo8KFyKDS1OLbmGKgRMuK2poadMJbr";
// pub const DNS_PATH: &str = "/dnsrbNCD66xjS3YC41cLTbRRxDz7pKCv4ylHpJHTkXzkO5mCoEvqgSTEPqYfLuJO425";

pub struct ServerCfg {
    pub pkcs12: String,
    pub listen_addr: String,
    pub pkcs12_password: String,
    pub dns_server_addr: String,
    pub uuid: String,
    pub external_addr: String,
    pub etcd_addr: String,
    pub etcd_user: String,
    pub etcd_password: String,
    pub token_key: String,
    pub my_grpc_addr: String,
    pub my_grpc_port: u16,

    pub tun_path: String,
    pub hub_grpc_addr: String,
}

impl ServerCfg {
    pub fn load_from_file(filepath: &str) -> Result<ServerCfg> {
        let mut file = File::open(&filepath)?;
        let mut jsonbytes = String::with_capacity(1024);
        file.read_to_string(&mut jsonbytes)?;

        use serde_json::Value;
        let v: Value = serde_json::from_str(&jsonbytes)?;

        let pkcs12 = match v["pkcs12"].as_str() {
            Some(t) => t.to_string(),
            None => "/home/abc/identity.pfx".to_string(),
        };

        let listen_addr = match v["listen_addr"].as_str() {
            Some(t) => t.to_string(),
            None => "0.0.0.0:8080".to_string(),
        };

        let pkcs12_password = match v["pkcs12_password"].as_str() {
            Some(t) => t.to_string(),
            None => "123456".to_string(),
        };

        let dns_server_addr = match v["dns_server_addr"].as_str() {
            Some(t) => t.to_string(),
            None => "1.1.1.1:53".to_string(),
        };

        let uuid = match v["uuid"].as_str() {
            Some(t) => t.to_string(),
            None => "".to_string(),
        };

        let external_addr = match v["external_addr"].as_str() {
            Some(t) => t.to_string(),
            None => "".to_string(),
        };

        let etcd_addr = match v["etcd_addr"].as_str() {
            Some(t) => t.to_string(),
            None => "".to_string(),
        };

        let etcd_user = match v["etcd_user"].as_str() {
            Some(t) => t.to_string(),
            None => "".to_string(),
        };

        let etcd_password = match v["etcd_password"].as_str() {
            Some(t) => t.to_string(),
            None => "".to_string(),
        };

        let tun_path = match v["tun_path"].as_str() {
            Some(t) => t.to_string(),
            None => "".to_string(),
        };

        let token_key = match v["token_key"].as_str() {
            Some(t) => t.to_string(),
            None => "".to_string(),
        };

        let hub_grpc_addr = match v["hub_grpc_addr"].as_str() {
            Some(t) => t.to_string(),
            None => "0.0.0.0".to_string(),
        };

        let my_grpc_addr = match v["my_grpc_addr"].as_str() {
            Some(t) => t.to_string(),
            None => "0.0.0.0".to_string(),
        };

        let my_grpc_port = match v["my_grpc_port"].as_u64() {
            Some(t) => t as u16,
            None => 8008,
        };

        let servercfg = ServerCfg {
            pkcs12,
            listen_addr,
            pkcs12_password,
            dns_server_addr,
            uuid,
            external_addr,
            etcd_addr,
            etcd_user,
            etcd_password,
            tun_path,
            token_key,
            my_grpc_addr,
            my_grpc_port,
            hub_grpc_addr,
        };

        if !servercfg.is_valid() {
            return Err(Error::new(ErrorKind::Other, "server cfg is invalid"));
        }

        Ok(servercfg)
    }

    fn is_valid(&self) -> bool {
        let mut valid = true;
        if self.uuid.len() < 1 {
            println!("server cfg invalid, uuid must provided");
            valid = false;
        }

        if self.token_key.len() < 1 {
            println!("server cfg invalid, token_key must provided");
            valid = false;
        }

        if self.etcd_addr.len() > 0 {
            if !self.etcd_addr.starts_with("https://") {
                println!("server cfg invalid, etcd server addr must begin with https");
                valid = false;
            }
        } else {
            // if no etcd_addr, the config file must provide tun path, dns tun path
            if self.tun_path.len() < 1 {
                println!("server cfg invalid, must provide tun_path");
                valid = false;
            }
        }

        return valid;
    }
}
