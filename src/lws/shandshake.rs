use bytes::{BufMut, BytesMut};
use futures::prelude::*;
use futures::try_ready;
use sha1::{Digest, Sha1};
use std::io::Error;
use tokio::prelude::*;

pub enum SHState {
    ReadingHeader,
    WritingResponse,
    Done,
}

pub struct SHandshake<T> {
    io: Option<T>,
    read_buf: BytesMut,
    wmsg: Option<super::WMessage>,
    state: SHState,
    path: Option<String>,
}

pub fn do_server_hanshake<T>(io: T) -> SHandshake<T> {
    SHandshake {
        io: Some(io),
        read_buf: BytesMut::with_capacity(1024),
        wmsg: None, //super::WMessage::new(write_buf.to_vec(), 0),
        state: SHState::ReadingHeader,
        path: None,
    }
}

/// Turns a Sec-WebSocket-Key into a Sec-WebSocket-Accept.
fn convert_key(input: &[u8]) -> String {
    // ... field is constructed by concatenating /key/ ...
    // ... with the string "258EAFA5-E914-47DA-95CA-C5AB0DC85B11" (RFC 6455)
    const WS_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let mut sh1 = Sha1::default();
    sh1.input(input);
    sh1.input(WS_GUID);
    base64::encode(&sh1.result())
}

impl<T> SHandshake<T> {
    pub fn parse_header(&mut self) -> bool {
        let needle = b"\r\n\r\n";
        let bm = &mut self.read_buf;
        if let Some(pos) = bm.windows(4).position(|window| window == needle) {
            let pos2 = pos + 4;
            let header = bm.split_to(pos2);
            let st = String::from_utf8_lossy(header.as_ref());
            //println!("recv header:\n{}", st);

            // extract path
            // GET /tunxxxxyyy?cap=1000 HTTP/1.1
            let begin_op = st.find("GET ");
            let end_op = st.find(" HTTP/1.1");
            if begin_op.is_some() && end_op.is_some() {
                let begin = begin_op.unwrap() + 4;
                let end = end_op.unwrap();
                let path = &st[begin..end];

                println!("path:{}", path);
                self.path = Some(path.to_string());
            }

            // extract key
            let mut key = "";
            let begin_op = st.find("Sec-WebSocket-Key ");
            if begin_op.is_some() {
                let begin = begin_op.unwrap() + 18;
                let mut end = begin;
                let strbytes = st.as_bytes();
                while strbytes[end] != b'\r' {
                    end += 1;
                }

                key = &st[begin..end];
            }

            let h = format!(
                "\
                 HTTP/1.1 101 Switching Protocols\r\n\
                 Connection: Upgrade\r\n\
                 Upgrade: websocket\r\n\
                 Sec-WebSocket-Accept: {}\r\n\
                 \r\n",
                convert_key(key.as_bytes()),
            );

            //println!("resp header:\n{}", h);
            let write_buf = h.as_bytes();
            let wmsg = super::WMessage::new(write_buf.to_vec(), 0);
            self.wmsg = Some(wmsg);

            return true;
        }

        false
    }
}

impl<T> Future for SHandshake<T>
where
    T: AsyncWrite + AsyncRead,
{
    type Item = (T, Option<String>);
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            match self.state {
                SHState::ReadingHeader => {
                    // read in
                    let io = self.io.as_mut().unwrap();
                    let bm = &mut self.read_buf;
                    if !bm.has_remaining_mut() {
                        // error, head too large
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            "header too large",
                        ));
                    }

                    let n = try_ready!(io.read_buf(bm));
                    if n == 0 {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            "can't read completed header",
                        ));
                    }

                    if self.parse_header() {
                        // completed
                        self.state = SHState::WritingResponse;
                    } else {
                        // continue loop
                    }
                }
                SHState::WritingResponse => {
                    // write out
                    let io = self.io.as_mut().unwrap();
                    let wmsg = self.wmsg.as_mut().unwrap();
                    try_ready!(io.write_buf(wmsg));

                    if wmsg.is_completed() {
                        self.state = SHState::Done;
                    }
                }
                SHState::Done => {
                    let io = self.io.take().unwrap();
                    let vv;
                    if self.path.is_some() {
                        vv = Some(self.path.take().unwrap());
                    } else {
                        vv = None;
                    }

                    return Ok(Async::Ready((io, vv)));
                }
            }
        }
    }
}
