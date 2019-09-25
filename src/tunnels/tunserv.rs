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
    let tunnel_cap = find_cap_from_query_string(&wsinfo.path);

    // Create a channel for our stream, which other sockets will use to
    // send us messages. Then register our address with the stream to send
    // data to us.
    let (tx, rx) = futures::sync::mpsc::unbounded();
    let rawfd = wsinfo.rawfd;
    let s2 = s.clone();
    let mut rf = s.borrow_mut();
    let t = Tunnel::new(
        rf.next_tunnel_id(),
        tx,
        rawfd,
        rf.dns_server_addr,
        tunnel_cap,
    );
    if let Err(_) = rf.on_tunnel_created(t.clone()) {
        // DROP all
        return;
    }

    let t2 = t.clone();
    let t3 = t.clone();
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

    let rx = rx.map_err(|_| {
        tungstenite::error::Error::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            "[tunbuilder] rx-shit",
        ))
    });

    let send_fut = rx.map(move |(req_idx, req_tag, msg)| {
        if req_idx != std::u16::MAX {
            t3.borrow_mut().flowctl_quota_increase(req_idx, req_tag);
        }
        msg
    });

    let send_fut = new_forward_ex(send_fut, sink, t4);

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

fn find_cap_from_query_string(path: &str) -> usize {
    if let Some(pos1) = path.find("?") {
        let substr = &path[pos1 + 1..];
        if let Some(pos2) = substr.find("cap=") {
            let param_str: &str;
            let substr = &substr[pos2 + 4..];
            if let Some(pos3) = substr.find("&") {
                param_str = &substr[..pos3];
            } else {
                param_str = substr;
            }

            if let Ok(t) = param_str.parse::<usize>() {
                if t > 1024 {
                    info!("[tunserv] tunnel cap from query string is too large:{}", t);
                    return 0;
                } else {
                    return t;
                }
            }
        }
    }

    0
}
