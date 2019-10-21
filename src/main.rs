mod config;
mod lws;
mod service;
mod tlsserver;
mod token;
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
    let mut filepath = String::default();
    if args.len() > 1 {
        let s = args.get(1).unwrap();
        if s == "-v" {
            println!("{}", VERSION);
            std::process::exit(0);
        } else if s == "-c" {
            if args.len() > 2 {
                let argstr = args.get(2).unwrap();
                filepath = argstr.to_string();
            }
        }
    }

    if filepath.len() < 1 {
        println!("please specify config file path with -c");
        std::process::exit(1);
    }

    info!("try to start lproxy-dv server, ver:{}", VERSION);

    // let handle = rt.handle();
    let tuncfg = match config::ServerCfg::load_from_file(&filepath) {
        Err(e) => {
            println!("load config file error: {}", e);
            std::process::exit(1);
        }
        Ok(t) => t,
    };

    let mut rt = Runtime::new().unwrap();

    let l = lazy(move || {
        let s = Service::new(tuncfg);
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
