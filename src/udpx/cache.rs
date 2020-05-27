use super::UStub;
use fnv::FnvHashMap as HashMap;
use futures::prelude::*;
use futures::ready;
use futures::task::{Context, Poll};
use log::info;
use std::cell::RefCell;
use std::pin::Pin;
use std::rc::Rc;
use std::time::Duration;
use tokio::time::Error;
use tokio::time::{delay_queue, DelayQueue};

pub type CacheKey = std::net::SocketAddr;
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
            is_in_keepalive: false,
        }))
    }

    pub fn insert(&mut self, ll: LongLiveC, key: CacheKey, value: UStub) {
        let delay = self
            .expirations
            .insert(key.clone(), Duration::from_secs(TTL_SECS));

        self.entries.insert(key, (value, delay));

        if !self.is_in_keepalive {
            self.task_keepalive(ll)
        }
    }

    pub fn get(&mut self, key: &CacheKey) -> Option<&mut UStub> {
        match self.entries.get_mut(key) {
            Some((ref mut v, ref k)) => {
                self.expirations.reset(k, Duration::from_secs(TTL_SECS));
                return Some(v);
            }
            _ => return None,
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
            let cf = CacheFuture::new(ll.clone());
            match cf.await {
                Err(_) => {}
                _ => {}
            }

            ll.borrow_mut().is_in_keepalive = false;
            info!("[Udpx-Cache] keepalive fut completed");
        };

        tokio::task::spawn_local(fut);
        self.is_in_keepalive = true;
        info!("[Udpx-Cache] keepalive start");
    }

    pub fn remove(&mut self, key: &CacheKey) {
        if let Some((_, cache_key)) = self.entries.remove(key) {
            self.expirations.remove(&cache_key);
        }
    }

    fn poll_purge(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        while let Some(res) = ready!(self.expirations.poll_expired(cx)) {
            let entry = res?;
            match self.entries.remove(entry.get_ref()) {
                Some((mut stub, _)) => {
                    stub.cleanup();
                }
                _ => {}
            }
        }

        Poll::Ready(Ok(()))
    }
}

struct CacheFuture {
    cache_ll: LongLiveC,
}

impl CacheFuture {
    pub fn new(cache_ll: LongLiveC) -> Self {
        CacheFuture { cache_ll }
    }
}

impl Future for CacheFuture {
    type Output = std::result::Result<(), Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let self_mut = self.get_mut();
        self_mut.cache_ll.borrow_mut().poll_purge(cx)
    }
}
