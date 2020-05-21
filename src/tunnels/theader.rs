use byte::*;
use std::convert::From;

pub const THEADER_SIZE: usize = 4;

#[derive(Debug)]
pub enum Cmd {
    None = 0,
    ReqData = 1,
    ReqCreated = 2,
    ReqClientClosed = 3,
    ReqClientFinished = 4,
    ReqServerFinished = 5,
    ReqServerClosed = 6,
    ReqClientQuota = 7,
    Ping = 8,
    Pong = 9,
    UdpX = 10,
}

#[derive(Debug)]
pub struct THeader {
    pub req_idx: u16,
    pub req_tag: u16,
}

impl THeader {
    pub fn new(idx: u16, tag: u16) -> THeader {
        THeader {
            req_idx: idx,
            req_tag: tag,
        }
    }

    pub fn read_from(bs: &[u8]) -> THeader {
        let offset = &mut 0;
        let req_idx = bs.read_with::<u16>(offset, LE).unwrap();
        let req_tag = bs.read_with::<u16>(offset, LE).unwrap();

        THeader::new(req_idx, req_tag)
    }

    pub fn write_to(&self, bs: &mut [u8]) {
        let offset = &mut 0;
        bs.write_with::<u16>(offset, self.req_idx, LE).unwrap();
        bs.write_with::<u16>(offset, self.req_tag, LE).unwrap();
    }
}

impl From<u8> for Cmd {
    fn from(v: u8) -> Self {
        match v {
            0 => Cmd::None,
            1 => Cmd::ReqData,
            2 => Cmd::ReqCreated,
            3 => Cmd::ReqClientClosed,
            4 => Cmd::ReqClientFinished,
            5 => Cmd::ReqServerFinished,
            6 => Cmd::ReqServerClosed,
            7 => Cmd::ReqClientQuota,
            8 => Cmd::Ping,
            9 => Cmd::Pong,
            10 => Cmd::UdpX,
            _ => panic!("unsupport {} to Cmd", v),
        }
    }
}
