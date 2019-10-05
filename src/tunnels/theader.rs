use byte::*;
use std::convert::From;

pub const THEADER_SIZE: usize = 5;

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
}

#[derive(Debug)]
pub struct THeader {
    pub cmd: u8,
    pub req_idx: u16,
    pub req_tag: u16,
}

impl THeader {
    pub fn new(cmd: Cmd, idx: u16, tag: u16) -> THeader {
        THeader {
            cmd: cmd as u8,
            req_idx: idx,
            req_tag: tag,
        }
    }

    pub fn new_data_header(idx: u16, tag: u16) -> THeader {
        THeader::new(Cmd::ReqData, idx, tag)
    }

    pub fn read_from(bs: &[u8]) -> THeader {
        let offset = &mut 0;
        let cmd = bs.read_with::<u8>(offset, LE).unwrap();
        let req_idx = bs.read_with::<u16>(offset, LE).unwrap();
        let req_tag = bs.read_with::<u16>(offset, LE).unwrap();

        THeader::new(Cmd::from(cmd), req_idx, req_tag)
    }

    pub fn write_to(&self, bs: &mut [u8]) {
        let offset = &mut 0;
        bs.write_with::<u8>(offset, self.cmd, LE).unwrap();
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
            _ => panic!("unsupport {} to Cmd", v),
        }
    }
}
