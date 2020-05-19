use crate::lws::WMessage;
use tokio::sync::mpsc::UnboundedSender;
use std::fmt;
use std::os::unix::io::RawFd;
use stream_cancel::Trigger;
use futures::task::Waker;

pub struct Request {
    pub index: u16,
    pub tag: u16,
    pub is_inused: bool,
    pub request_tx: Option<UnboundedSender<WMessage>>,
    pub trigger: Option<Trigger>,

    // pub ipv4_le: u32,
    // pub port_le: u16,
    pub quota: u32,

    pub wait_task: Option<Waker>,

    pub rawfd: Option<RawFd>,
}

impl Request {
    pub fn new(idx: u16) -> Request {
        Request {
            index: idx,
            tag: 0,
            request_tx: None,
            trigger: None,
            // ipv4_le: 0,
            // port_le: 0,
            is_inused: false,
            quota: 0,
            wait_task: None,
            rawfd: None,
        }
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Req {{ indx: {}, tag: {} }}", self.index, self.tag,)
    }
}
