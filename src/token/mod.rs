use aes::Aes128;
use cfb_mode::Cfb;
use std::time::{SystemTime, UNIX_EPOCH};

use std::io::{Error, ErrorKind, Result};
type AesCfb = Cfb<Aes128>;
use cfb_mode::stream_cipher::{NewStreamCipher, StreamCipher};

const TOKEN_TIMEOUT: u64 = 30 * 24 * 60 * 60;
const TOKEN_KEY: &[u8] = b"@yymmxxkk#$yzilm";

fn decrypt(token: &str) -> Result<String> {
    let b64 = base64::decode_config(token, base64::URL_SAFE);
    if b64.is_err() {
        return Err(Error::new(
            ErrorKind::Other,
            format!("base64 decode failed:{}", b64.err().unwrap()),
        ));
    }

    let mut b64 = b64.unwrap();
    let iv = (&b64[0..16]).to_vec(); // block size is 16
    let buffer = &mut b64[16..];

    let mut cfb = AesCfb::new_var(TOKEN_KEY, &iv[..]).unwrap();
    cfb.decrypt(buffer);

    Ok(String::from_utf8_lossy(buffer).to_string())
}

pub fn token_decode(token: &str) -> Result<String> {
    if token.len() < 1 {
        return Err(Error::new(ErrorKind::Other, "token is empty"));
    }

    let uuid_with_timestamp = decrypt(token)?;

    let v: Vec<&str> = uuid_with_timestamp.split('@').collect();
    if v.len() != 2 {
        return Err(Error::new(ErrorKind::Other, "token format is invalid"));
    }

    let timestamp = v[1];
    let timestamp: u64 = timestamp.parse().unwrap_or(0);

    let start = SystemTime::now();
    let since_the_epoch = start
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards");
    let in_ms = since_the_epoch.as_secs();

    if in_ms > timestamp && (in_ms - timestamp) > TOKEN_TIMEOUT {
        return Err(Error::new(ErrorKind::Other, "token has expired"));
    }

    Ok(v[0].to_string())
}
