use super::LongLiveTM;
use super::Tunnel;
use crate::tlsserver::WSStreamInfo;
// use futures::sink::Sink;
use futures::{Future, Stream};
use log::{debug, info}; // error
use tokio;
// use tokio::prelude::*;
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
    // let t3 = t.clone();
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
    // let send_fut = rx.fold(sink, move |mut sink, (req_idx, req_tag, item)| {
    //     // flowctl_quota_increase
    //     if req_idx != std::u16::MAX {
    //         t3.borrow_mut().flowctl_quota_increase(req_idx, req_tag);
    //     }

    //     let ss = sink.start_send(item);
    //     let start_send_error;
    //     if let Err(e) = ss {
    //         error!("[tunserv] start_send error:{}", e);
    //         start_send_error = Some(e);
    //     } else {
    //         start_send_error = None;
    //     }

    //     // wait sink send completed
    //     WebsocketSinkCtl::new(sink, start_send_error).map_err(|_| ())
    // });

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

// struct WebsocketSinkCtl<T>
// where
//     T: Sink,
// {
//     sink: Option<T>,
//     sink_error: Option<T::SinkError>,
// }

// impl<T> WebsocketSinkCtl<T>
// where
//     T: Sink,
// {
//     pub fn new(sink: T, sink_error: Option<T::SinkError>) -> Self {
//         WebsocketSinkCtl {
//             sink: Some(sink),
//             sink_error,
//             // start_sink,
//         }
//     }
// }

// impl<T> Future for WebsocketSinkCtl<T>
// where
//     T: Sink,
// {
//     type Item = T;
//     type Error = T::SinkError;

//     fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
//         if self.sink_error.is_some() {
//             return Err(self.sink_error.take().unwrap());
//         }

//         let result = self.sink.as_mut().unwrap().poll_complete();
//         match result {
//             Ok(Async::Ready(_)) => return Ok(Async::Ready(self.sink.take().unwrap())),
//             Ok(Async::NotReady) => return Ok(Async::NotReady),
//             Err(e) => {
//                 return Err(e);
//             }
//         }
//     }
// }

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
