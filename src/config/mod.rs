mod logsimple;
pub use logsimple::*;
use std::fs::File;
use std::io::{Read, Result};

pub const KEEP_ALIVE_INTERVAL: u64 = 15 * 1000;

pub const TUN_PATH: &str = "/tunxKMZ2oFFxixut7cO4nI1Q8pNqNHjGfvobrFo8KFyKDS1OLbmGKgRMuK2poadMJbr";
pub const DNS_PATH: &str = "/dnsrbNCD66xjS3YC41cLTbRRxDz7pKCv4ylHpJHTkXzkO5mCoEvqgSTEPqYfLuJO425";

pub struct ServerCfg {
    pub pkcs12: String,
    pub listen_addr: String,
    pub pkcs12_password: String,
    pub dns_server_addr: String,
    pub uuid: String,
    pub external_addr: String,
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

        let tuncfg = ServerCfg {
            pkcs12,
            listen_addr,
            pkcs12_password,
            dns_server_addr,
            uuid,
            external_addr,
        };

        Ok(tuncfg)
    }
}
