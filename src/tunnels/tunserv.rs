use super::Tunnel;
use super::{LongLiveTM, LongLiveUA, UserAccount};
use crate::tlsserver::WSStreamInfo;
use futures::prelude::*;
use log::{debug, info};
use tokio;

pub fn serve_websocket(
    wsinfo: WSStreamInfo,
    tun_quota: u32,
    accref: &mut UserAccount,
    account: LongLiveUA,
    s: LongLiveTM,
) {
    let mut wsinfo = wsinfo;
    let ws_stream = wsinfo.ws.take().unwrap();

    let mut tunnel_cap = find_usize_from_query_string(&wsinfo.path, "cap=");
    let mut tunnel_req_quota: u32 = find_usize_from_query_string(&wsinfo.path, "quota=") as u32;

    if tunnel_cap > 1024 {
        info!(
            "[tunserv] tunnel cap from query string is too large:{}, reset to 0",
            tunnel_cap
        );
        tunnel_cap = 0;
    }

    if tunnel_req_quota == 0 {
        tunnel_req_quota = std::u32::MAX;
    }

    let s2 = s.clone();
    let mut rf = s.borrow_mut();
    let is_dns = wsinfo.is_dns;

    // Create a channel for our stream, which other sockets will use to
    // send us messages. Then register our address with the stream to send
    // data to us.
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let rawfd = wsinfo.rawfd;

    let t = Tunnel::new(
        rf.next_tunnel_id(),
        tx,
        rawfd,
        rf.dns_server_addr,
        tunnel_cap,
        tunnel_req_quota,
        tun_quota,
        account,
        is_dns,
    );

    if let Err(_) = rf.on_tunnel_created(accref, t.clone()) {
        // DROP all
        return;
    }

    let t2 = t.clone();

    // `sink` is the stream of messages going out.
    // `stream` is the stream of incoming messages.
    let (sink, stream) = ws_stream.split();

    let receive_fut = stream.for_each(move |message| {
        debug!("[tunserv]tunnel read a message");
        match message {
            Ok(m)=> {
                // post to manager
                let mut clone = t.borrow_mut();
                clone.on_tunnel_msg(m, t.clone());
            }
            _ => {}
        }

        future::ready(())
    });

    let rx = rx.map(|x|{Ok(x)});
    let send_fut = rx.forward(super::SinkEx::new(sink, t2.clone())); // TODO:

    // Wait for one future to complete.
    let select_fut = async move {
        future::select(receive_fut, send_fut).await;
        info!("[tunserv] both websocket futures completed");
        let mut rf = s2.borrow_mut();
        rf.on_tunnel_closed(t2.clone());
    };

    tokio::task::spawn_local(select_fut);
}

fn find_usize_from_query_string(path: &str, key: &str) -> usize {
    if let Some(pos1) = path.find("?") {
        let substr = &path[pos1 + 1..];
        if let Some(pos2) = substr.find(key) {
            let param_str: &str;
            let substr = &substr[(pos2 + key.len())..];
            if let Some(pos3) = substr.find("&") {
                param_str = &substr[..pos3];
            } else {
                param_str = substr;
            }

            if let Ok(t) = param_str.parse::<usize>() {
                return t;
            }
        }
    }

    0
}

pub fn find_str_from_query_string(path: &str, key: &str) -> String {
    if let Some(pos1) = path.find("?") {
        let substr = &path[pos1 + 1..];
        if let Some(pos2) = substr.find(key) {
            let param_str: &str;
            let substr = &substr[(pos2 + key.len())..];
            if let Some(pos3) = substr.find("&") {
                param_str = &substr[..pos3];
            } else {
                param_str = substr;
            }

            return param_str.to_string();
        }
    }

    String::default()
}
