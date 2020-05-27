use super::{TMessage, WMessage};
use futures::prelude::*;
use futures::ready;
use futures::stream::FusedStream;
use futures::task::{Context, Poll};
use std::io::Error;
use std::pin::Pin;
use tokio::io::{AsyncRead, AsyncWrite};

pub struct TcpFramed<T> {
    io: T,
    reading: Option<TMessage>,
    writing: Option<WMessage>,
    has_finished: bool,
}

impl<T> TcpFramed<T> {
    pub fn new(io: T) -> Self {
        TcpFramed {
            io,
            reading: None,
            writing: None,
            has_finished: false,
        }
    }
}

impl<T> Stream for TcpFramed<T>
where
    T: AsyncRead + Unpin,
{
    type Item = std::result::Result<TMessage, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let self_mut = self.get_mut();
        loop {
            //self.inner.poll()
            if self_mut.reading.is_none() {
                self_mut.reading = Some(TMessage::new());
            }

            let reading = &mut self_mut.reading;
            let msg = reading.as_mut().unwrap();

            // read from io
            let mut io = &mut self_mut.io;
            let pin_io = Pin::new(&mut io);
            match ready!(pin_io.poll_read_buf(cx, msg)) {
                Ok(n) => {
                    if n <= 0 {
                        self_mut.has_finished = true;
                        return Poll::Ready(None);
                    }
                }
                Err(e) => {
                    self_mut.has_finished = true;
                    return Poll::Ready(Some(Err(e)));
                }
            };

            return Poll::Ready(Some(Ok(self_mut.reading.take().unwrap())));
        }
    }
}

impl<T> FusedStream for TcpFramed<T>
where
    T: AsyncRead + Unpin,
{
    fn is_terminated(&self) -> bool {
        self.has_finished
    }
}

impl<T> Sink<WMessage> for TcpFramed<T>
where
    T: AsyncWrite + Unpin,
{
    type Error = Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.writing.is_some() {
            return self.poll_flush(cx);
        }

        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: WMessage) -> Result<(), Self::Error> {
        let self_mut = self.get_mut();
        self_mut.writing = Some(item);
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let self_mut = self.get_mut();
        if self_mut.writing.is_some() {
            let writing = self_mut.writing.as_mut().unwrap();
            loop {
                let mut io = &mut self_mut.io;
                let pin_io = Pin::new(&mut io);
                ready!(pin_io.poll_write_buf(cx, writing))?;

                if writing.is_completed() {
                    self_mut.writing = None;
                    break;
                }
            }
        }

        let mut io = &mut self_mut.io;
        let pin_io = Pin::new(&mut io);
        // Try flushing the underlying IO
        ready!(pin_io.poll_flush(cx))?;

        return Poll::Ready(Ok(()));
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self.poll_flush(cx))?;
        Poll::Ready(Ok(()))
    }
}
