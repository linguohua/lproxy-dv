mod config;
mod server;
mod service;
mod tunnels;

use futures::future::lazy;
use futures::stream::Stream;
use futures::Future;
use log::{error, info};
use service::Service;
use signal_hook::iterator::Signals;
use std::env;
use tokio::runtime::current_thread::Runtime;

const VERSION: &'static str = env!("CARGO_PKG_VERSION");

fn main() {
    config::log_init().unwrap();
    let args: Vec<String> = env::args().collect();
    // println!("{:?}", args);
    if args.len() > 1 {
        let s = args.get(1).unwrap();
        if s == "-v" {
            println!("{}", VERSION);
            std::process::exit(0);
        }
    }

    info!("try to start lproxy-dv server..");
    let mut rt = Runtime::new().unwrap();
    // let handle = rt.handle();

    let l = lazy(|| {
        let s = Service::new();
        s.borrow_mut().start(s.clone());

        let wait_signal = Signals::new(&[signal_hook::SIGUSR1, signal_hook::SIGUSR2])
            .unwrap()
            .into_async()
            .unwrap()
            .into_future()
            .map(move |sig| {
                info!("got sigal:{:?}", sig.0);
                // Service::stop
                if sig.0 == Some(signal_hook::SIGUSR1) {
                    s.borrow_mut().stop();
                } else if sig.0 == Some(signal_hook::SIGUSR2) {
                    // just for test
                    s.borrow_mut().restart();
                }

                ()
            })
            .map_err(|e| error!("{}", e.0));

        wait_signal
    });

    rt.spawn(l);
    rt.run().unwrap();
}
