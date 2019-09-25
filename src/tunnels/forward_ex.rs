use super::Tunnel;
use futures::sink::Sink;
use futures::stream::{Fuse, Stream};
use futures::{try_ready, Async, AsyncSink, Future, Poll};
use std::cell::RefCell;
use std::rc::Rc;
use tungstenite::protocol::Message;

/// Future for the `Stream::forward` combinator, which sends a stream of values
/// to a sink and then waits until the sink has fully flushed those values.
#[must_use = "futures do nothing unless polled"]
pub struct ForwardEx<T: Stream, U> {
    sink: Option<U>,
    stream: Option<Fuse<T>>,
    buffered: Option<Message>,
    tun: Rc<RefCell<Tunnel>>,
    has_flowctl: bool,
}

pub fn new_forward_ex<T, U>(stream: T, sink: U, tun: Rc<RefCell<Tunnel>>) -> ForwardEx<T, U>
where
    U: Sink<SinkItem = Message>,
    T: Stream<Item = Message>,
    T::Error: From<U::SinkError>,
{
    let has_flowctl = tun.borrow().has_flowctl;
    ForwardEx {
        sink: Some(sink),
        stream: Some(stream.fuse()),
        buffered: None,
        tun,
        has_flowctl,
    }
}

impl<T, U> ForwardEx<T, U>
where
    U: Sink<SinkItem = Message>,
    T: Stream<Item = Message>,
    T::Error: From<U::SinkError>,
{
    /// Get a shared reference to the inner sink.
    /// If this combinator has already been polled to completion, None will be returned.
    // pub fn sink_ref(&self) -> Option<&U> {
    //     self.sink.as_ref()
    // }

    /// Get a mutable reference to the inner sink.
    /// If this combinator has already been polled to completion, None will be returned.
    pub fn sink_mut(&mut self) -> Option<&mut U> {
        self.sink.as_mut()
    }

    /// Get a shared reference to the inner stream.
    /// If this combinator has already been polled to completion, None will be returned.
    // pub fn stream_ref(&self) -> Option<&T> {
    //     self.stream.as_ref().map(|x| x.get_ref())
    // }

    /// Get a mutable reference to the inner stream.
    /// If this combinator has already been polled to completion, None will be returned.
    pub fn stream_mut(&mut self) -> Option<&mut T> {
        self.stream.as_mut().map(|x| x.get_mut())
    }

    fn take_result(&mut self) -> (T, U) {
        let sink = self
            .sink
            .take()
            .expect("Attempted to poll ForwardEx after completion");
        let fuse = self
            .stream
            .take()
            .expect("Attempted to poll ForwardEx after completion");
        (fuse.into_inner(), sink)
    }

    fn try_start_send(&mut self, item: Message) -> Poll<(), U::SinkError> {
        debug_assert!(self.buffered.is_none());
        if let AsyncSink::NotReady(item) = self
            .sink_mut()
            .expect("Attempted to poll ForwardEx after completion")
            .start_send(item)?
        {
            self.buffered = Some(item);
            return Ok(Async::NotReady);
        }
        Ok(Async::Ready(()))
    }
}

impl<T, U> Future for ForwardEx<T, U>
where
    U: Sink<SinkItem = Message>,
    T: Stream<Item = Message>,
    T::Error: From<U::SinkError>,
{
    type Item = (T, U);
    type Error = T::Error;

    fn poll(&mut self) -> Poll<(T, U), T::Error> {
        let mut bytes_cosume = 0;
        // If we've got an item buffered already, we need to write it to the
        // sink before we can do anything else
        if let Some(item) = self.buffered.take() {
            bytes_cosume = item.len();
            try_ready!(self.try_start_send(item))
        }

        loop {
            // TODO: flowctl
            if self.has_flowctl {
                match self.tun.borrow_mut().poll_tunnel_quota_with(bytes_cosume) {
                    Err(_) => {}
                    Ok(t) => {
                        if !t {
                            return Ok(Async::NotReady);
                        }
                    }
                }
            }

            match self
                .stream_mut()
                .expect("Attempted to poll ForwardEx after completion")
                .poll()?
            {
                Async::Ready(Some(item)) => {
                    bytes_cosume = item.len();
                    try_ready!(self.try_start_send(item))
                }
                Async::Ready(None) => {
                    try_ready!(self
                        .sink_mut()
                        .expect("Attempted to poll ForwardEx after completion")
                        .close());
                    return Ok(Async::Ready(self.take_result()));
                }
                Async::NotReady => {
                    try_ready!(self
                        .sink_mut()
                        .expect("Attempted to poll ForwardEx after completion")
                        .poll_complete());
                    return Ok(Async::NotReady);
                }
            }
        }
    }
}

// pub trait StreamForwardEx: Stream {
//     fn forward_ex<U>(self, sink: U, tun: Rc<RefCell<Tunnel>>) -> ForwardEx<Self, U>
//     where
//         Self: Stream<Item = Message>,
//         U: Sink<SinkItem = Message>,
//         Self::Error: From<U::SinkError>,
//         Self: Sized,
//     {
//         new_forward_ex(self, sink, tun)
//     }
// }

// impl<S> StreamForwardEx for S where S: Stream<Item = Message> {}
