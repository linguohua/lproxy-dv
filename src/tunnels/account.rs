use crate::config::QUOTA_RESET_INTERVAL;
use futures::task::Task;
use std::cell::RefCell;
use std::rc::Rc;
pub type LongLiveUA = Rc<RefCell<UserAccount>>;

pub struct UserAccount {
    pub uuid: String,
    pub quota_remain: usize,
    pub quota_per_second: usize,
    wait_tasks: Vec<Task>,
    pub refcount: usize,
}

impl UserAccount {
    pub fn new(uuid: String, quota_per_second_in_kbytes: usize) -> LongLiveUA {
        let quota_per_second = quota_per_second_in_kbytes * 1000;
        let quota_remain = quota_per_second * (QUOTA_RESET_INTERVAL as usize / 1000);

        let v = UserAccount {
            uuid,
            quota_remain,
            quota_per_second,
            wait_tasks: Vec::with_capacity(20),
            refcount: 0,
        };

        Rc::new(RefCell::new(v))
    }

    pub fn reset_quota(&mut self) {
        self.quota_remain = self.quota_per_second * (QUOTA_RESET_INTERVAL as usize / 1000);
        if self.wait_tasks.len() > 0 {
            loop {
                match self.wait_tasks.pop() {
                    Some(t) => {
                        t.notify();
                    }
                    None => {
                        return;
                    }
                }
            }
        }
    }

    pub fn consume(&mut self, bytes_count: usize) -> usize {
        if self.quota_remain > bytes_count {
            self.quota_remain = self.quota_remain - bytes_count;
        } else {
            self.quota_remain = 0;
        }

        self.quota_remain
    }

    pub fn poll_tunnel_quota_with(&mut self, bytes_cosume: usize) -> Result<bool, ()> {
        let remain = self.consume(bytes_cosume);

        if remain == 0 {
            self.wait_tasks.push(futures::task::current());
            return Ok(false);
        }

        Ok(true)
    }

    pub fn set_quota_if(&mut self, quota_per_second_in_kbytes: usize) {
        if quota_per_second_in_kbytes == 0 {
            return;
        }

        let quota_per_second = quota_per_second_in_kbytes * 1000;
        self.quota_per_second = quota_per_second;
        self.quota_remain = quota_per_second * (QUOTA_RESET_INTERVAL as usize / 1000);
    }

    pub fn has_flowctl(&self) -> bool {
        return self.quota_per_second > 0;
    }
}
