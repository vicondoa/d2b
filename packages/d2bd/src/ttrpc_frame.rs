use std::io::ErrorKind;

use tokio::io::{AsyncRead, AsyncReadExt};
use ttrpc::proto::{MESSAGE_HEADER_LENGTH, MessageHeader};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TtrpcFrameError {
    Io,
    Truncated,
    Oversize,
}

pub(crate) async fn read_ttrpc_frame<R>(
    reader: &mut R,
    negotiated_limit: u32,
) -> Result<Option<(MessageHeader, Vec<u8>)>, TtrpcFrameError>
where
    R: AsyncRead + Unpin,
{
    let logical_limit = negotiated_limit
        .min(d2b_contracts::v2_component_session::MAX_LOGICAL_MESSAGE_BYTES)
        as usize;
    let mut header_bytes = [0_u8; MESSAGE_HEADER_LENGTH];
    match reader.read(&mut header_bytes[..1]).await {
        Ok(0) => return Ok(None),
        Ok(1) => {}
        Ok(_) => unreachable!("one-byte read returned more than one byte"),
        Err(_) => return Err(TtrpcFrameError::Io),
    }
    if let Err(error) = reader.read_exact(&mut header_bytes[1..]).await {
        return Err(if error.kind() == ErrorKind::UnexpectedEof {
            TtrpcFrameError::Truncated
        } else {
            TtrpcFrameError::Io
        });
    }
    let header = MessageHeader::from(header_bytes);
    let body_len = header.length as usize;
    if body_len > logical_limit {
        return Err(TtrpcFrameError::Oversize);
    }
    let mut body = vec![0_u8; body_len];
    if let Err(error) = reader.read_exact(&mut body).await {
        return Err(if error.kind() == ErrorKind::UnexpectedEof {
            TtrpcFrameError::Truncated
        } else {
            TtrpcFrameError::Io
        });
    }
    let mut frame = Vec::with_capacity(MESSAGE_HEADER_LENGTH + body_len);
    frame.extend_from_slice(&header_bytes);
    frame.extend_from_slice(&body);
    Ok(Some((header, frame)))
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncWriteExt;

    use super::*;

    fn frame(stream_id: u32, body: &[u8]) -> Vec<u8> {
        let mut frame = Vec::from(MessageHeader::new_response(
            stream_id,
            u32::try_from(body.len()).unwrap(),
        ));
        frame.extend_from_slice(body);
        frame
    }

    #[tokio::test]
    async fn reads_fragmented_header_and_body_as_one_frame() {
        let expected = frame(7, b"payload");
        let (mut writer, mut reader) = tokio::io::duplex(64);
        let outbound = expected.clone();
        let send = tokio::spawn(async move {
            for byte in outbound {
                writer.write_all(&[byte]).await.unwrap();
                tokio::task::yield_now().await;
            }
        });
        let (_, actual) = read_ttrpc_frame(&mut reader, 64).await.unwrap().unwrap();
        send.await.unwrap();
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn separates_coalesced_frames() {
        let first = frame(1, b"one");
        let second = frame(2, b"two");
        let (mut writer, mut reader) = tokio::io::duplex(128);
        let mut combined = first.clone();
        combined.extend_from_slice(&second);
        writer.write_all(&combined).await.unwrap();
        assert_eq!(
            read_ttrpc_frame(&mut reader, 64).await.unwrap().unwrap().1,
            first
        );
        assert_eq!(
            read_ttrpc_frame(&mut reader, 64).await.unwrap().unwrap().1,
            second
        );
    }

    #[tokio::test]
    async fn rejects_truncated_header_and_body() {
        let (mut writer, mut reader) = tokio::io::duplex(64);
        writer.write_all(&[1, 2, 3]).await.unwrap();
        writer.shutdown().await.unwrap();
        assert_eq!(
            read_ttrpc_frame(&mut reader, 64).await,
            Err(TtrpcFrameError::Truncated)
        );

        let (mut writer, mut reader) = tokio::io::duplex(64);
        let complete = frame(1, b"payload");
        writer
            .write_all(&complete[..complete.len() - 1])
            .await
            .unwrap();
        writer.shutdown().await.unwrap();
        assert_eq!(
            read_ttrpc_frame(&mut reader, 64).await,
            Err(TtrpcFrameError::Truncated)
        );
    }

    #[tokio::test]
    async fn rejects_declared_body_over_negotiated_limit() {
        let (mut writer, mut reader) = tokio::io::duplex(64);
        writer
            .write_all(&Vec::from(MessageHeader::new_response(1, 65)))
            .await
            .unwrap();
        assert_eq!(
            read_ttrpc_frame(&mut reader, 64).await,
            Err(TtrpcFrameError::Oversize)
        );
    }
}
