use super::{TMessage, WMessage};
use futures::prelude::*;
use futures::try_ready;
use std::io::Error;
use tokio::io::AsyncRead;
use tokio::prelude::*;

pub struct TcpFramed<T> {
    io: T,
    reading: Option<TMessage>,
    writing: Option<WMessage>,
}

impl<T> TcpFramed<T> {
    pub fn new(io: T) -> Self {
        TcpFramed {
            io,
            reading: None,
            writing: None,
        }
    }
}

impl<T> Stream for TcpFramed<T>
where
    T: AsyncRead,
{
    type Item = TMessage;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        loop {
            //self.inner.poll()
            if self.reading.is_none() {
                self.reading = Some(TMessage::new());
            }

            let reading = &mut self.reading;
            let msg = reading.as_mut().unwrap();

            // read from io
            let n = try_ready!(self.io.read_buf(msg));
            if n == 0 {
                return Ok(Async::Ready(None));
            }

            return Ok(Async::Ready(Some(self.reading.take().unwrap())));
        }
    }
}

impl<T> Sink for TcpFramed<T>
where
    T: AsyncWrite,
{
    type SinkItem = WMessage;
    type SinkError = Error;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        if self.writing.is_some() {
            self.poll_complete()?;

            if self.writing.is_some() {
                return Ok(AsyncSink::NotReady(item));
            }
        }

        self.writing = Some(item);
        Ok(AsyncSink::Ready)
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        if self.writing.is_some() {
            loop {
                let writing = self.writing.as_mut().unwrap();
                try_ready!(self.io.write_buf(writing));

                if writing.is_completed() {
                    self.writing = None;
                    break;
                }
            }
        }

        // Try flushing the underlying IO
        try_ready!(self.io.poll_flush());

        return Ok(Async::Ready(()));
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        try_ready!(self.poll_complete());

        Ok(self.io.shutdown()?)
    }
}
