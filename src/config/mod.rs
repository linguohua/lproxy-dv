mod logsimple;
pub use logsimple::*;

pub const CFG_MONITOR_INTERVAL:u64 = 5*60*1000;
pub const KEEP_ALIVE_INTERVAL:u64 = 15*1000;

pub struct TunCfg {

}

impl TunCfg {
    pub fn new() -> TunCfg {
        TunCfg{}
    }
}
