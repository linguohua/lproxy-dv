mod config;
mod service;
mod tunnels;
mod server;

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

        let wait_signal = Signals::new(&[signal_hook::SIGUSR1])
            .unwrap()
            .into_async()
            .unwrap()
            .into_future()
            .map(move |sig| {
                info!("got sigal:{:?}", sig.0);
                // Service::stop
                s.borrow_mut().stop();
                ()
            })
            .map_err(|e| error!("{}", e.0));

        wait_signal
    });

    rt.spawn(l);
    rt.run().unwrap();
}
