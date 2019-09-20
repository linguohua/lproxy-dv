use bytes::Bytes;
use futures::sync::mpsc::UnboundedSender;
use std::fmt;
use stream_cancel::Trigger;

pub struct Request {
    pub index: u16,
    pub tag: u16,
    pub is_inused: bool,
    pub request_tx: Option<UnboundedSender<Bytes>>,
    pub trigger: Option<Trigger>,

    pub ipv4_le: u32,
    pub port_le: u16,
}

impl Request {
    pub fn new(idx: u16) -> Request {
        Request {
            index: idx,
            tag: 0,
            request_tx: None,
            trigger: None,
            ipv4_le: 0,
            port_le: 0,
            is_inused: false,
        }
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Req {{ indx: {}, tag: {}, ip:{}, port:{} }}",
            self.index, self.tag, self.ipv4_le, self.port_le
        )
    }
}
