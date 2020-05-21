use std::net::SocketAddr;
use tokio::time::{delay_queue, DelayQueue};
use super::UStub;
use futures::prelude::*;
use fnv::FnvHashMap as HashMap;
use std::time::Duration;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AddressPair {
    pub src_addr:SocketAddr,
    pub dst_addr: SocketAddr,
}

pub type CacheKey = AddressPair;
pub type LongLiveC = Rc<RefCell<Cache>>;
pub struct Cache {
    entries: HashMap<CacheKey, (UStub, delay_queue::Key)>,
    expirations: DelayQueue<CacheKey>,
    is_in_keepalive: bool,
}

const TTL_SECS: u64 = 60;

impl Cache {
    pub fn new() -> LongLiveC {
        Rc::new(RefCell::new(Cache {
            entries: HashMap::default(),
            expirations: DelayQueue::new(),
            is_in_keepalive:false,
        }))
    }
    
    pub fn insert(&mut self, ll: LongLiveC, key: CacheKey, value: UStub) {
        let delay = self.expirations
            .insert(key.clone(), Duration::from_secs(TTL_SECS));

        self.entries.insert(key, (value, delay));

        if !self.is_in_keepalive {
            self.task_keepalive(ll)
        }
    }

    pub fn get(&mut self, key: &CacheKey) -> Option<&UStub> {
        match self.entries.get(key) {
            Some((ref v, ref k)) => {
                self.expirations.reset(k, Duration::from_secs(TTL_SECS));
                return Some(v)
            }
            _ => {
                return None
            }
        }
    }

    // pub fn cleanup(&mut self) {
    //     self.expirations.clear();
    //     for (_, mut v) in self.entries.drain() {
    //         v.0.cleanup(); // ustub cleanup
    //     }
    // }

    fn task_keepalive(&mut self, ll: LongLiveC) {
        let fut = async move {
            while let Some(entry) = ll.borrow_mut().expirations.next().await {
                match entry {
                    Ok(entry) => {
                        match ll.borrow_mut().entries.remove(entry.get_ref()) {
                            Some((mut stub, _)) => {
                                stub.cleanup();
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }

            ll.borrow_mut().is_in_keepalive = false;
        };

        tokio::task::spawn_local(fut);
        self.is_in_keepalive = true;
    }

    pub fn remove(&mut self, key: &CacheKey) {
        if let Some((_, cache_key)) = self.entries.remove(key) {
            self.expirations.remove(&cache_key);
        }
    }
}
