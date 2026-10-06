//! Length-prefixed postcard frames.
//!
//! A frame is a 4-byte big-endian length `n`, then `n` bytes: one
//! protocol-version byte ([`FRAME_VERSION`]) and the postcard encoding of the
//! message. Readers refuse frames larger than their limit before allocating,
//! so a hostile peer cannot make us reserve gigabytes with a 4-byte header.

use serde::{de::DeserializeOwned, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Version byte at the start of every frame body.
pub const FRAME_VERSION: u8 = 1;

/// Default largest frame (1 MiB).
pub const MAX_FRAME: usize = 1 << 20;

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame of {len} bytes is larger than the limit of {max} bytes")]
    TooLarge { len: usize, max: usize },
    #[error("empty frame")]
    Empty,
    #[error("unsupported frame version {0}")]
    Version(u8),
    #[error("could not decode message: {0}")]
    Decode(postcard::Error),
    #[error("could not encode message: {0}")]
    Encode(postcard::Error),
}

/// Encodes `msg` as one frame body (version byte + postcard).
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, FrameError> {
    let mut body = vec![FRAME_VERSION];
    body.extend(postcard::to_stdvec(msg).map_err(FrameError::Encode)?);
    Ok(body)
}

/// Decodes a frame body made by [`encode`].
pub fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T, FrameError> {
    match body.split_first() {
        None => Err(FrameError::Empty),
        Some((&FRAME_VERSION, rest)) => postcard::from_bytes(rest).map_err(FrameError::Decode),
        Some((&v, _)) => Err(FrameError::Version(v)),
    }
}

/// Writes one frame. Fails if the encoded message is larger than `max`.
pub async fn write_frame<W, T>(w: &mut W, msg: &T, max: usize) -> Result<(), FrameError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let body = encode(msg)?;
    if body.len() > max {
        return Err(FrameError::TooLarge {
            len: body.len(),
            max,
        });
    }
    let len = u32::try_from(body.len()).map_err(|_| FrameError::TooLarge {
        len: body.len(),
        max,
    })?;
    w.write_all(&len.to_be_bytes()).await?;
    w.write_all(&body).await?;
    Ok(())
}

/// Reads one frame of at most `max` bytes.
pub async fn read_frame<R, T>(r: &mut R, max: usize) -> Result<T, FrameError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    if len > max {
        return Err(FrameError::TooLarge { len, max });
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    decode(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize, serde::Deserialize, Debug, PartialEq)]
    enum Msg {
        A(u32),
        B { bytes: Vec<u8> },
    }

    #[tokio::test]
    async fn round_trip() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        write_frame(&mut a, &Msg::A(7), MAX_FRAME).await.unwrap();
        write_frame(
            &mut a,
            &Msg::B {
                bytes: vec![1, 2, 3],
            },
            MAX_FRAME,
        )
        .await
        .unwrap();
        assert_eq!(
            read_frame::<_, Msg>(&mut b, MAX_FRAME).await.unwrap(),
            Msg::A(7)
        );
        assert_eq!(
            read_frame::<_, Msg>(&mut b, MAX_FRAME).await.unwrap(),
            Msg::B {
                bytes: vec![1, 2, 3]
            }
        );
    }

    #[tokio::test]
    async fn refuses_oversized_header_before_allocating() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
        let err = read_frame::<_, Msg>(&mut b, MAX_FRAME).await.unwrap_err();
        assert!(matches!(err, FrameError::TooLarge { .. }), "{err}");
    }

    #[tokio::test]
    async fn refuses_oversized_write() {
        let (mut a, _b) = tokio::io::duplex(64);
        let err = write_frame(
            &mut a,
            &Msg::B {
                bytes: vec![0; 100],
            },
            50,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, FrameError::TooLarge { .. }));
    }

    #[test]
    fn rejects_unknown_version() {
        let mut body = encode(&Msg::A(1)).unwrap();
        body[0] = 9;
        assert!(matches!(decode::<Msg>(&body), Err(FrameError::Version(9))));
        assert!(matches!(decode::<Msg>(&[]), Err(FrameError::Empty)));
    }
}
