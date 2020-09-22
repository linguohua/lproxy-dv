use super::Tunnel;
use super::{LongLiveTM, LongLiveUD, UserDevice};
use crate::tlsserver::{WSStreamInfo, WSStream, LwsHTTP, LwsTLS};
use futures::prelude::*;
use log::{debug, error, info};
use tokio;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use crate::lws;

pub fn serve_websocket(
    wsinfo: WSStreamInfo,
    accref: &mut UserDevice,
    device: LongLiveUD,
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
    let is_dns = wsinfo.is_dns;

    // Create a channel for our stream, which other sockets will use to
    // send us messages. Then register our address with the stream to send
    // data to us.
    let (tx, rx) = unbounded_channel();
    let rawfd = wsinfo.rawfd;

    let t = {
        let mut rf = s.borrow_mut();
        Tunnel::new(
            rf.next_tunnel_id(),
            tx,
            rawfd,
            rf.dns_server_addr,
            tunnel_cap,
            tunnel_req_quota,
            device,
            is_dns,
        )
    };

    let has_flowctl;
    if is_dns {
        has_flowctl = false;
    } else {
        has_flowctl = accref.has_flowctl();
    }

    {
        let mut rf = s.borrow_mut();
        if let Err(_) = rf.on_tunnel_created(accref, t.clone()) {
            // DROP all
            return;
        }
    }

    let t2 = t.clone();

    // TODO: use generic function to reduce duplicate code
    match ws_stream {
        WSStream::HTTPWSStream(hws) => {
            serve_wsstream_http(hws, s2, t2, rx, has_flowctl);
        },
        WSStream::TlsWSStream(tls) => {
            serve_wsstream_tls(tls, s2, t2, rx, has_flowctl);
        }
    };
}

fn serve_wsstream_tls (ws: LwsTLS, s: LongLiveTM, t: super::LongLiveTun, rx: UnboundedReceiver<lws::WMessage>, has_flowctl: bool) {
    // `sink` is the stream of messages going out.
    // `stream` is the stream of incoming messages.
    let (sink, mut stream) = ws.split();

    let t2 = t.clone();

    let receive_fut = async move {
        while let Some(message) = stream.next().await {
            debug!("[tunserv]tunnel read a message");
            match message {
                Ok(m) => {
                    // post to manager
                    let mut clone = t.borrow_mut();
                    clone.on_tunnel_msg(m, t.clone());
                }
                Err(e) => {
                    error!("[tunserv]stream.next error:{}", e);
                    break;
                }
            }
        }
    };

    let rx = rx.map(|x| Ok(x));
    let send_fut = rx.forward(super::SinkEx::new(sink, t2.clone(), has_flowctl)); // TODO:

    // Wait for one future to complete.
    let select_fut = async move {
        future::select(receive_fut.boxed_local(), send_fut).await;
        info!("[tunserv] both websocket futures completed");
        let mut rf = s.borrow_mut();
        rf.on_tunnel_closed(t2.clone());
    };

    tokio::task::spawn_local(select_fut);
}

fn serve_wsstream_http (ws: LwsHTTP, s: LongLiveTM, t: super::LongLiveTun, rx: UnboundedReceiver<lws::WMessage>, has_flowctl: bool) {
    // `sink` is the stream of messages going out.
    // `stream` is the stream of incoming messages.
    let (sink, mut stream) = ws.split();

    let t2 = t.clone();

    let receive_fut = async move {
        while let Some(message) = stream.next().await {
            debug!("[tunserv]tunnel read a message");
            match message {
                Ok(m) => {
                    // post to manager
                    let mut clone = t.borrow_mut();
                    clone.on_tunnel_msg(m, t.clone());
                }
                Err(e) => {
                    error!("[tunserv]stream.next error:{}", e);
                    break;
                }
            }
        }
    };

    let rx = rx.map(|x| Ok(x));
    let send_fut = rx.forward(super::SinkEx::new(sink, t2.clone(), has_flowctl)); // TODO:

    // Wait for one future to complete.
    let select_fut = async move {
        future::select(receive_fut.boxed_local(), send_fut).await;
        info!("[tunserv] both websocket futures completed");
        let mut rf = s.borrow_mut();
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
