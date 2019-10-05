mod logsimple;
pub use logsimple::*;

pub const KEEP_ALIVE_INTERVAL: u64 = 5 * 1000;
pub const WEBSOCKET_QUEUE_SIZE: usize = 64;

pub const TUN_PATH: &str = "/tunxKMZ2oFFxixut7cO4nI1Q8pNqNHjGfvobrFo8KFyKDS1OLbmGKgRMuK2poadMJbr";
pub const DNS_PATH: &str = "/dnsrbNCD66xjS3YC41cLTbRRxDz7pKCv4ylHpJHTkXzkO5mCoEvqgSTEPqYfLuJO425";

pub struct TunCfg {
    pub pkcs12: String,
    pub listen_addr: String,
    pub pkcs12_password: String,
    pub dns_server_addr: String,
}

impl TunCfg {
    pub fn new() -> TunCfg {
        TunCfg {
            //pkcs12: "/home/abc/identity.pfx".to_string(),
            pkcs12: "/etc/v2ray/identity.pfx".to_string(),
            pkcs12_password: "123456".to_string(),
            //listen_addr: "127.0.0.1:12345".to_string(),
            //dns_server_addr: "223.5.5.5:53".to_string(),
            listen_addr: "0.0.0.0:8080".to_string(),
            dns_server_addr: "1.1.1.1:53".to_string(),
        }
    }
}
