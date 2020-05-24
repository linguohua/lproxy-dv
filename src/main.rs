mod config;
mod lws;
mod myrpc;
mod service;
mod tlsserver;
mod token;
mod tunnels;
mod udpx;

use log::{info};
use service::Service;
use std::env;
use tokio::signal::unix::{signal, SignalKind};
use tokio::runtime;

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

    let mut basic_rt = runtime::Builder::new()
    .basic_scheduler()
    .enable_all()
    .build().unwrap();
    // let handle = rt.handle();
    let local = tokio::task::LocalSet::new();

    let l = async move {
        let s = Service::new(tuncfg);
        s.borrow_mut().start(s.clone());
        let s2 = s.clone();

        let lx = async move {
            let mut ss = signal(SignalKind::user_defined2()).unwrap();
            while let Some(x) = ss.recv().await {
                println!("got signal {:?}", x);
                s2.borrow_mut().restart();
            }
        };
        tokio::task::spawn_local(lx);

        let mut ss = signal(SignalKind::user_defined1()).unwrap();
        // TODO: user_defined2, restart
        let sig = ss.recv().await;
        println!("got signal {:?}", sig);
            // Service::stop
        s.borrow_mut().stop();
    };

    local.spawn_local(l);
    basic_rt.block_on(local);
}
