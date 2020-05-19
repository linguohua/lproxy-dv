use super::Tunnel;
use crate::lws::WMessage;
use futures::sink::Sink;
// use futures::{try_ready, Async, AsyncSink, Future, Poll};
use std::cell::RefCell;
use std::rc::Rc;
use std::pin::Pin;
use futures::task::{Context, Poll};
use futures::ready;

/// Future for the `Stream::forward` combinator, which sends a stream of values
/// to a sink and then waits until the sink has fully flushed those values.
#[must_use = "futures do nothing unless polled"]
pub struct SinkEx<T> {
    sink: T,
    bytes_consume: u64,
    tun: Rc<RefCell<Tunnel>>,
    has_flowctl: bool,
}

impl<T> SinkEx<T>
where
    T: Sink<WMessage>,
{
    pub fn new(sink: T, tun: Rc<RefCell<Tunnel>>) -> Self
    {
        let has_flowctl = tun.borrow().has_flowctl;
        SinkEx {
            sink,
            bytes_consume:0,
            tun,
            has_flowctl,
        }
    }
}

impl<T> Sink<WMessage> for SinkEx<T>
where
    T: Sink<WMessage>+Unpin
{
    type Error = T::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // Poll::Ready(Ok(()))
        let self_mut = self.get_mut();
        // check flowctl

        if self_mut.has_flowctl {
            let bytes_consume = self_mut.bytes_consume;
            self_mut.bytes_consume = 0;

            match self_mut
                .tun
                .borrow()
                .poll_tunnel_quota_with(bytes_consume as usize, cx.waker().clone())
            {
                Err(_) => {}
                Ok(t) => {
                    if !t {
                        return Poll::Pending;
                    }
                }
            }
        }

        let mut io = &mut self_mut.sink;
        let pin_io = Pin::new(&mut io);
        pin_io.poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: WMessage) -> Result<(), Self::Error> {
        let self_mut = self.get_mut();
        
        self_mut.bytes_consume = item.content_length as u64;
        let mut io = &mut self_mut.sink;
        let pin_io = Pin::new(&mut io);
        pin_io.start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let self_mut = self.get_mut();

        let mut io = &mut self_mut.sink;
        let pin_io = Pin::new(&mut io);
        // Try flushing the underlying IO
        pin_io.poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self.poll_flush(cx))?;
        Poll::Ready(Ok(()))
    }
}
