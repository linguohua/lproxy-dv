mod logsimple;
pub use logsimple::*;

pub const CFG_MONITOR_INTERVAL:u64 = 5*60*1000;
pub const KEEP_ALIVE_INTERVAL:u64 = 15*1000;
pub const PER_TCP_QUOTA:u32 = 100;

pub struct TunCfg {
    pub pkcs12: String,
    pub listen_addr:String,
    pub pkcs12_password: String,
    pub dns_server_addr: String,
}

impl TunCfg {
    pub fn new() -> TunCfg {
        TunCfg{
            pkcs12: "/home/abc/identity.pfx".to_string(),
            pkcs12_password:"123456".to_string(),
            listen_addr: "127.0.0.1:12345".to_string(),
            dns_server_addr: "223.5.5.5:53".to_string(),
        }
    }
}
