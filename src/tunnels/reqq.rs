use super::Request;
use crate::config::PER_TCP_QUOTA;
use log::error;
use nix::sys::socket::{shutdown, Shutdown};

pub struct Reqq {
    pub elements: Vec<Request>,
}

impl Reqq {
    pub fn new(size: usize) -> Reqq {
        let mut elements = Vec::with_capacity(size);
        for n in 0..size {
            elements.push(Request::new(n as u16));
        }

        Reqq { elements: elements }
    }

    pub fn alloc(&mut self, req_idx: u16, req_tag: u16, port: u16, ip: u32) {
        let elements = &mut self.elements;
        if (req_idx as usize) >= elements.len() {
            error!("[Reqq] alloc failed, req_idx exceed");
            return;
        }

        let req = &mut elements[req_idx as usize];
        Reqq::clean_req(req);

        req.tag = req_tag;
        req.ipv4_le = ip;
        req.port_le = port;
        req.request_tx = None;
        req.trigger = None;
        req.quota = PER_TCP_QUOTA;
        req.is_inused = true;
    }

    pub fn free(&mut self, idx: u16, tag: u16) -> bool {
        let elements = &mut self.elements;
        if idx as usize >= elements.len() {
            return false;
        }

        let req = &mut elements[idx as usize];

        if req.tag != tag {
            return false;
        }

        if !req.is_inused {
            return false;
        }

        Reqq::clean_req(req);

        true
    }

    pub fn clear_all(&mut self) {
        let elements = &mut self.elements;
        for e in elements.iter_mut() {
            Reqq::clean_req(e);
        }
    }

    fn clean_req(req: &mut Request) {
        // debug!("[Reqq]clean_req:{:?}", req.tag);
        req.request_tx = None;
        req.trigger = None;
        req.is_inused = false;

        if req.wait_task.is_some() {
            let wait_task = req.wait_task.take().unwrap();
            wait_task.notify();
        }

        if req.rawfd.is_some() {
            let rawfd = req.rawfd.take().unwrap();
            let r = shutdown(rawfd, Shutdown::Both);
            match r {
                Err(e) => {
                    error!("[Reqq]close_rawfd failed:{}", e);
                }
                _ => {}
            }
        }
    }
}
