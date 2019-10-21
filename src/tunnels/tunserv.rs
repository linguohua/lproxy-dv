use super::new_forward_ex;
use super::LongLiveTM;
use super::Tunnel;
use crate::tlsserver::WSStreamInfo;
use futures::{Future, Stream};
use log::{debug, info};
use tokio;
use tokio::runtime::current_thread;

pub fn serve_websocket(wsinfo: WSStreamInfo, s: LongLiveTM) {
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
    let token_key = rf.token_key.as_bytes();

    let quota_per_second_in_kbytes = find_usize_from_query_string(&wsinfo.path, "limit=");
    let token_str = find_str_from_query_string(&wsinfo.path, "tok=");
    let uuidof = crate::token::token_decode(&token_str, token_key);
    let uuid;
    if uuidof.is_err() {
        info!("[tunserv] decode token error:{}", uuidof.err().unwrap());
        uuid = String::default();
    } else {
        uuid = uuidof.unwrap();
    }

    // Create a channel for our stream, which other sockets will use to
    // send us messages. Then register our address with the stream to send
    // data to us.
    let (tx, rx) = futures::sync::mpsc::unbounded();
    let rawfd = wsinfo.rawfd;

    let t = Tunnel::new(
        rf.next_tunnel_id(),
        tx,
        rawfd,
        rf.dns_server_addr,
        tunnel_cap,
        tunnel_req_quota,
        quota_per_second_in_kbytes,
        uuid,
    );

    if let Err(_) = rf.on_tunnel_created(t.clone()) {
        // DROP all
        return;
    }

    let t2 = t.clone();
    // let t3 = t.clone();
    let t4 = t.clone();

    // `sink` is the stream of messages going out.
    // `stream` is the stream of incoming messages.
    let (sink, stream) = ws_stream.split();

    let receive_fut = stream.for_each(move |message| {
        debug!("[tunserv]tunnel read a message");
        // post to manager
        let mut clone = t.borrow_mut();
        clone.on_tunnel_msg(message, t.clone());

        Ok(())
    });

    let rx = rx.map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "[tunbuilder] rx-shit"));

    let send_fut = new_forward_ex(rx, sink, t4);

    // Wait for either of futures to complete.
    let receive_fut = receive_fut
        .map(|_| ())
        .map_err(|_| ())
        .select(send_fut.map(|_| ()).map_err(|_| ()))
        .then(move |_| {
            info!("[tunserv] both websocket futures completed");
            let mut rf = s2.borrow_mut();
            rf.on_tunnel_closed(t2.clone());
            Ok(())
        });

    current_thread::spawn(receive_fut);
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

fn find_str_from_query_string(path: &str, key: &str) -> String {
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
