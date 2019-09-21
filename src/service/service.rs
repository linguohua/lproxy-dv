use crate::config::{self};
use futures::sync::mpsc::UnboundedSender;
use log::{debug, error, info};
use std::fmt;
use std::time::{Duration, Instant};
use stream_cancel::{StreamExt, Trigger, Tripwire};
use tokio::prelude::*;
use tokio::runtime::current_thread;
use tokio::timer::Delay;
const STATE_STOPPED: u8 = 0;
const STATE_STARTING: u8 = 1;
const STATE_RUNNING: u8 = 2;
const STATE_STOPPING: u8 = 3;
use super::SubServiceCtl;
use std::cell::RefCell;
use std::rc::Rc;

type LongLive = Rc<RefCell<Service>>;

enum Instruction {
    StartSubServices,
    Restart,
}

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s;
        match self {
            Instruction::StartSubServices => s = "StartSubServices",
            Instruction::Restart => s = "Restart",
        }
        write!(f, "({})", s)
    }
}

type TxType = UnboundedSender<Instruction>;

pub struct Service {
    state: u8,
    subservices: Vec<SubServiceCtl>,
    ins_tx: Option<TxType>,
    tuncfg: Option<std::sync::Arc<config::TunCfg>>,
    instruction_trigger: Option<Trigger>,
}

impl Service {
    pub fn new() -> LongLive {
        Rc::new(RefCell::new(Service {
            subservices: Vec::new(),
            ins_tx: None,
            tuncfg: None,
            instruction_trigger: None,
            state: 0,
        }))
    }

    // start config monitor
    pub fn start(&mut self, s: LongLive) {
        if self.state == STATE_STOPPED {
            self.state = STATE_STARTING;

            let (tx, rx) = futures::sync::mpsc::unbounded();
            let (trigger, tripwire) = Tripwire::new();
            self.save_instruction_trigger(trigger);

            let clone = s.clone();
            let fut = rx
                .take_until(tripwire)
                .for_each(move |ins| {
                    Service::process_instruction(clone.clone(), ins);
                    Ok(())
                })
                .then(|_| {
                    info!("[Service] instruction rx future completed");

                    Ok(())
                });

            self.save_tx(Some(tx));
            let cfg = config::TunCfg::new();
            self.save_cfg(cfg);
            self.fire_instruction(Instruction::StartSubServices);

            current_thread::spawn(fut);
        } else {
            panic!("[Service] start failed, state not stopped");
        }
    }

    pub fn stop(&mut self) {
        info!("[Service]stop");
        if self.state != STATE_RUNNING {
            error!("[Service] do_restart failed, state not running");
            return;
        }

        self.state = STATE_STOPPING;

        // drop trigger will completed instruction future
        self.instruction_trigger = None;

        super::cleanup_subservices(&mut self.subservices);

        self.subservices.clear();

        self.state = STATE_STOPPED;
    }

    pub fn restart(&mut self) {
        self.fire_instruction(Instruction::Restart);
    }

    fn process_instruction(s: LongLive, ins: Instruction) {
        match ins {
            Instruction::StartSubServices => {
                Service::do_start_subservices(s.clone());
            }
            Instruction::Restart => {
                Service::do_restart(s.clone());
            }
        }
    }

    fn delay_post_instruction(s: LongLive, seconds: u64, ins: Instruction) {
        info!(
            "[Service]delay_post_instruction, seconds:{}, ins:{}",
            seconds, ins
        );

        // delay 5 seconds
        let when = Instant::now() + Duration::from_millis(seconds * 1000);
        let task = Delay::new(when)
            .and_then(move |_| {
                debug!("[Service]delay_post_instruction retry...");
                s.borrow().fire_instruction(ins);

                Ok(())
            })
            .map_err(|e| {
                error!(
                    "[Service]delay_post_instruction delay retry errored, err={:?}",
                    e
                );
                ()
            });

        current_thread::spawn(task);
    }

    fn do_start_subservices(s: LongLive) {
        info!("[Service]do_start_subservices");
        let cfg;
        {
            cfg = s.borrow().tuncfg.as_ref().unwrap().clone();
        }

        let clone = s.clone();
        let clone2 = s.clone();
        let fut = super::start_subservice(cfg)
            .and_then(move |subservices| {
                let s2 = &mut clone.borrow_mut();
                let vec_subservices = &mut subservices.borrow_mut();
                while let Some(ctl) = vec_subservices.pop() {
                    s2.subservices.push(ctl);
                }

                s2.state = STATE_RUNNING;

                Ok(())
            })
            .or_else(|_| {
                Service::delay_post_instruction(clone2, 50, Instruction::StartSubServices);
                Err(())
            });

        current_thread::spawn(fut);
    }

    fn do_restart(s1: LongLive) {
        info!("[Service]do_restart");
        let mut s = s1.borrow_mut();
        s.stop();
        s.start(s1.clone());
    }

    fn save_tx(&mut self, tx: Option<TxType>) {
        self.ins_tx = tx;
    }

    fn save_cfg(&mut self, cfg: config::TunCfg) {
        self.tuncfg = Some(std::sync::Arc::new(cfg));
    }

    fn save_instruction_trigger(&mut self, trigger: Trigger) {
        self.instruction_trigger = Some(trigger);
    }

    fn fire_instruction(&self, ins: Instruction) {
        debug!("[Service]fire_instruction, ins:{}", ins);
        let tx = &self.ins_tx;
        match tx.as_ref() {
            Some(ref tx) => {
                if let Err(e) = tx.unbounded_send(ins) {
                    error!("[Service]fire_instruction failed:{}", e);
                }
            }
            None => {
                error!("[Service]fire_instruction failed: no tx");
            }
        }
    }
}
