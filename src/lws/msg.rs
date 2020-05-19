use core::mem::MaybeUninit;
use byte::*;
use bytes::{Buf, BufMut};
use std::slice;

pub const DEFAULT_CAP: usize = 8096;
pub const HEAD_SIZE: usize = 2;

pub struct RMessage {
    pub buf: Option<Vec<u8>>,
    pub cached_legnth: u16,
}

impl RMessage {
    pub fn new() -> Self {
        let buf = Vec::with_capacity(DEFAULT_CAP);
        RMessage {
            buf: Some(buf),
            cached_legnth: 0,
        }
    }

    pub fn is_completed(&self) -> bool {
        let vec = self.buf.as_ref().unwrap();
        self.cached_legnth > 0 && self.cached_legnth == (vec.len() as u16)
    }

    // unsafe fn as_raw(&mut self) -> &mut [u8] {
    //     let vec = self.buf.as_mut().unwrap();
    //     let ptr = vec.as_mut_ptr();
    //     &mut slice::from_raw_parts_mut(ptr, DEFAULT_CAP)[..]
    // }
}

impl BufMut for RMessage {
    fn remaining_mut(&self) -> usize {
        let vec = self.buf.as_ref().unwrap();
        let remain;
        // if no header read
        if vec.len() < HEAD_SIZE {
            remain = (HEAD_SIZE - vec.len()) as usize;
        } else {
            remain = (self.cached_legnth as usize - vec.len()) as usize;
        }

        //println!("remaining_mut:{}", remain);

        return remain;
    }

    unsafe fn advance_mut(&mut self, cnt: usize) {
        let vec = self.buf.as_mut().unwrap();
        let nlen = cnt + vec.len();

        vec.set_len(nlen);
        //println!("advance_mut, cnt:{}, vlen:{}", cnt, vec.len());

        if self.cached_legnth == 0 && nlen >= HEAD_SIZE {
            let bs = &vec[..HEAD_SIZE];
            let offset = &mut 0;
            self.cached_legnth = bs.read_with::<u16>(offset, LE).unwrap();

            if self.cached_legnth == 0 || self.cached_legnth > (DEFAULT_CAP as u16) {
                panic!("cached_legnth not valid:{}", self.cached_legnth);
            }
            // println!("cached_legnth: {}", self.cached_legnth);
        }
    }

    fn bytes_mut(&mut self) -> &mut [MaybeUninit<u8>] {
        let len = self.remaining_mut();
        let vec = self.buf.as_mut().unwrap();
        let begin = vec.len();
        unsafe {
            let ptr = vec.as_mut_ptr().offset(begin as isize);
            //MaybeUninit::new(&mut x[begin..end])
            slice::from_raw_parts_mut(ptr as *mut MaybeUninit<u8>, len)
        }
    }
}

pub struct WMessage {
    pub buf: Option<Vec<u8>>,
    pub cursor: u16,
    pub content_length: u16,
}

impl WMessage {
    pub fn new(v: Vec<u8>, cursor: u16) -> Self {
        let ll = v.len();
        WMessage {
            buf: Some(v),
            cursor,
            content_length: ll as u16 - cursor,
        }
    }

    pub fn is_completed(&self) -> bool {
        let vec = self.buf.as_ref().unwrap();
        self.cursor == vec.len() as u16
    }
}

impl Buf for WMessage {
    fn remaining(&self) -> usize {
        let vec = self.buf.as_ref().unwrap();
        vec.len() - self.cursor as usize
    }

    fn bytes(&self) -> &[u8] {
        let vec = self.buf.as_ref().unwrap();
        &vec[self.cursor as usize..]
    }

    fn advance(&mut self, cnt: usize) {
        self.cursor += cnt as u16;
    }
}

pub struct TMessage {
    pub buf: Option<Vec<u8>>,
}

const TMESSAGE_KEEP: usize = 7;
impl TMessage {
    pub fn new() -> Self {
        let mut buf = Vec::with_capacity(DEFAULT_CAP);
        for _ in 0..TMESSAGE_KEEP {
            buf.push(0);
        }

        TMessage { buf: Some(buf) }
    }

    // unsafe fn as_raw(&mut self) -> &mut [u8] {
    //     let vec = self.buf.as_mut().unwrap();
    //     let ptr = vec.as_mut_ptr();
    //     &mut slice::from_raw_parts_mut(ptr, DEFAULT_CAP)[..]
    // }
}

impl BufMut for TMessage {
    fn remaining_mut(&self) -> usize {
        let vec = self.buf.as_ref().unwrap();
        let remain = vec.capacity() - vec.len();

        return remain;
    }

    unsafe fn advance_mut(&mut self, cnt: usize) {
        let vec = self.buf.as_mut().unwrap();
        let nlen = cnt + vec.len();

        vec.set_len(nlen);
        //println!("advance_mut, cnt:{}, vlen:{}", cnt, vec.len());
    }

    fn bytes_mut(&mut self) -> &mut [MaybeUninit<u8>] {
        let len = self.remaining_mut();
        let vec = self.buf.as_mut().unwrap();
        let begin = vec.len();
        unsafe {
            let ptr = vec.as_mut_ptr().offset(begin as isize);
            //MaybeUninit::new(&mut x[begin..end])
            slice::from_raw_parts_mut(ptr as *mut MaybeUninit<u8>, len)
        }
    }
}
