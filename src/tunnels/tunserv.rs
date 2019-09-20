use super::LongLiveTM;
use super::Tunnel;
use crate::server::WSStreamInfo;
use futures::{Future, Stream};
use log::{debug, info};
use tokio;
use tokio::runtime::current_thread;

pub fn serve_websocket(wsinfo: WSStreamInfo, s: LongLiveTM) {
    let mut wsinfo = wsinfo;
    let ws_stream = wsinfo.ws.take().unwrap();

    // Create a channel for our stream, which other sockets will use to
    // send us messages. Then register our address with the stream to send
    // data to us.
    let (tx, rx) = futures::sync::mpsc::unbounded();
    let rawfd = wsinfo.rawfd;
    let s2 = s.clone();
    let mut rf = s.borrow_mut();
    let t = Tunnel::new(rf.next_tunnel_id(), tx, rawfd, rf.is_dns_tun);
    if let Err(_) = rf.on_tunnel_created(t.clone()) {
        // DROP all
        return;
    }

    let t2 = t.clone();
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
            "[tunserv] rx-shit",
        ))
    });

    let send_fut = rx.forward(sink);

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
