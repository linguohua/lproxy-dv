use super::{RMessage, WMessage};
use bytes::BufMut;
use futures::prelude::*;
use futures::try_ready;
use std::io::Error;
use tokio::io::AsyncRead;
use tokio::prelude::*;

pub struct LwsFramed<T> {
    io: T,
    reading: Option<RMessage>,
    writing: Option<WMessage>,
    tail: Option<Vec<u8>>,
}

impl<T> LwsFramed<T> {
    pub fn new(io: T, tail: Option<Vec<u8>>) -> Self {
        LwsFramed {
            io,
            reading: None,
            writing: None,
            tail,
        }
    }
}

fn read_from_tail<B: BufMut>(vec: &mut Vec<u8>, bf: &mut B) -> Vec<u8> {
    let remain_in_bf = bf.remaining_mut();
    let len_in_vec = vec.len();
    let len_to_copy = if len_in_vec < remain_in_bf {
        len_in_vec
    } else {
        remain_in_bf
    };

    bf.put_slice(&vec[..len_to_copy]);
    vec.split_off(len_to_copy)
}

impl<T> Stream for LwsFramed<T>
where
    T: AsyncRead,
{
    type Item = RMessage;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        loop {
            //self.inner.poll()
            if self.reading.is_none() {
                self.reading = Some(RMessage::new());
            }

            let reading = &mut self.reading;
            let msg = reading.as_mut().unwrap();

            if self.tail.is_some() {
                // has tail, handle tail first
                // self.read_from_tail(msg);
                let tail = &mut self.tail;
                let tail = tail.as_mut().unwrap();

                let tail = read_from_tail(tail, msg);
                if tail.len() < 1 {
                    self.tail = None;
                } else {
                    self.tail = Some(tail);
                }
            } else {
                // read from io
                let n = try_ready!(self.io.read_buf(msg));
                if n == 0 {
                    return Ok(Async::Ready(None));
                }
            }

            if msg.is_completed() {
                // if message is completed
                // return ready
                return Ok(Async::Ready(Some(self.reading.take().unwrap())));
            }
        }
    }
}

impl<T> Sink for LwsFramed<T>
where
    T: AsyncWrite,
{
    type SinkItem = WMessage;
    type SinkError = Error;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        if self.writing.is_some() {
            return Ok(AsyncSink::NotReady(item));
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
