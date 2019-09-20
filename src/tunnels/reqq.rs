use super::Request;

use log::error;

pub struct Reqq {
    pub elements: Vec<Request>,
}

impl Reqq {
    pub fn new(size: usize) -> Reqq {
        let mut elements = Vec::with_capacity(size);
        for n in 0..size {
            elements.push(Request::new(n as u16));
        }

        Reqq {
            elements: elements,
        }
    }

    pub fn alloc(&mut self, req_idx:u16, req_tag:u16, port:u16, ip:u32) {
        let elements = &mut self.elements;
        if (req_idx as usize) >= elements.len() {
            error!("[Reqq] alloc failed, req_idx exceed");
            return;
        }

        let req = &mut elements[req_idx as usize];
        req.tag = req_tag;
        req.ipv4_le = ip;
        req.port_le = port;
        req.request_tx = None;
        req.trigger = None;

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
    }
}
