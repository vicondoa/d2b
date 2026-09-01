//! Client-side ttrpc bridge for an authenticated ComponentSession.
//!
//! Generated ttrpc clients expect an ordinary byte stream. ComponentSession
//! owns the authenticated transport and request registry, so this adapter
//! translates the two framing layers without exposing the session driver to
//! generated service code.

use std::sync::Arc;

use d2b_contracts_zone_session::v3::component_session::MAX_LOGICAL_MESSAGE_BYTES;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    task::JoinHandle,
};
use ttrpc::{
    r#async::{Client, transport::Socket},
    proto::{
        MESSAGE_HEADER_LENGTH, MESSAGE_TYPE_DATA, MESSAGE_TYPE_REQUEST, MESSAGE_TYPE_RESPONSE,
        MessageHeader,
    },
};

use crate::{Cancellation, ComponentSessionDriver, SessionError, ttrpc_request_id};

const CLIENT_BRIDGE_CAPACITY: usize = 256 * 1024;

/// A generated-ttrpc client bound to one authenticated ComponentSession.
///
/// The returned [`Client`] clone is suitable for generated service clients.
/// Keeping this value alive keeps the bridge alive and ensures the session
/// request registry is cleaned up when the client is dropped.
pub struct SessionTtrpcClient {
    client: Client,
    bridge: Option<JoinHandle<()>>,
    cancellation: Cancellation,
}

impl std::fmt::Debug for SessionTtrpcClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SessionTtrpcClient(<redacted>)")
    }
}

impl SessionTtrpcClient {
    /// Create a generated-ttrpc client over an established session driver.
    pub fn new(driver: Arc<dyn ComponentSessionDriver>) -> Self {
        let (client_transport, bridge_transport) = tokio::io::duplex(CLIENT_BRIDGE_CAPACITY);
        let client = Client::new(Socket::new(client_transport));
        let cancellation = Cancellation::new();
        let bridge_cancellation = cancellation.clone();
        let bridge = tokio::spawn(async move {
            let (reader, writer) = tokio::io::split(bridge_transport);
            let outgoing =
                forward_client_frames(reader, Arc::clone(&driver), bridge_cancellation.clone());
            let incoming =
                forward_session_frames(writer, Arc::clone(&driver), bridge_cancellation.clone());
            tokio::select! {
                _ = outgoing => {}
                _ = incoming => {}
            }
            bridge_cancellation.cancel();
        });
        Self {
            client,
            bridge: Some(bridge),
            cancellation,
        }
    }

    /// Clone the underlying generated-ttrpc client.
    pub fn client(&self) -> Client {
        self.client.clone()
    }
}

impl Drop for SessionTtrpcClient {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(bridge) = self.bridge.take() {
            bridge.abort();
        }
    }
}

async fn forward_client_frames<R>(
    mut reader: R,
    driver: Arc<dyn ComponentSessionDriver>,
    cancellation: Cancellation,
) -> Result<(), SessionClientBridgeError>
where
    R: AsyncRead + Unpin,
{
    loop {
        let frame = read_frame(&mut reader, &cancellation).await?;
        match frame_type(&frame)? {
            MESSAGE_TYPE_REQUEST => {
                let request_id = ttrpc_request_id(driver.generation(), &frame)
                    .map_err(|_| SessionClientBridgeError::Frame)?;
                driver
                    .start_ttrpc(request_id, frame)
                    .await
                    .map_err(SessionClientBridgeError::Session)?;
            }
            MESSAGE_TYPE_DATA => {
                driver
                    .send_ttrpc(frame)
                    .await
                    .map_err(SessionClientBridgeError::Session)?;
            }
            _ => return Err(SessionClientBridgeError::Frame),
        }
    }
}

async fn forward_session_frames<W>(
    mut writer: W,
    driver: Arc<dyn ComponentSessionDriver>,
    cancellation: Cancellation,
) -> Result<(), SessionClientBridgeError>
where
    W: AsyncWrite + Unpin,
{
    loop {
        let frame = tokio::select! {
            _ = cancellation.cancelled() => return Ok(()),
            result = driver.receive_ttrpc() => result.map_err(SessionClientBridgeError::Session)?,
        };
        let kind = frame_type(&frame)?;
        if !matches!(kind, MESSAGE_TYPE_DATA | MESSAGE_TYPE_RESPONSE) {
            return Err(SessionClientBridgeError::Frame);
        }
        writer
            .write_all(&frame)
            .await
            .map_err(|_| SessionClientBridgeError::Transport)?;
        writer
            .flush()
            .await
            .map_err(|_| SessionClientBridgeError::Transport)?;
        if kind == MESSAGE_TYPE_RESPONSE {
            let request_id = ttrpc_request_id(driver.generation(), &frame)
                .map_err(|_| SessionClientBridgeError::Frame)?;
            driver
                .complete_ttrpc(request_id)
                .await
                .map_err(SessionClientBridgeError::Session)?;
        }
    }
}

async fn read_frame<R>(
    reader: &mut R,
    cancellation: &Cancellation,
) -> Result<Vec<u8>, SessionClientBridgeError>
where
    R: AsyncRead + Unpin,
{
    let mut header_bytes = [0_u8; MESSAGE_HEADER_LENGTH];
    tokio::select! {
        _ = cancellation.cancelled() => return Err(SessionClientBridgeError::Cancelled),
        result = reader.read_exact(&mut header_bytes) => {
            result.map_err(|_| SessionClientBridgeError::Transport)?;
        }
    }
    let header = MessageHeader::from(header_bytes);
    let body_len = usize::try_from(header.length).map_err(|_| SessionClientBridgeError::Frame)?;
    if body_len > MAX_LOGICAL_MESSAGE_BYTES as usize {
        return Err(SessionClientBridgeError::Frame);
    }
    let mut frame = Vec::with_capacity(
        MESSAGE_HEADER_LENGTH
            .checked_add(body_len)
            .ok_or(SessionClientBridgeError::Frame)?,
    );
    frame.extend_from_slice(&header_bytes);
    frame.resize(MESSAGE_HEADER_LENGTH + body_len, 0);
    tokio::select! {
        _ = cancellation.cancelled() => return Err(SessionClientBridgeError::Cancelled),
        result = reader.read_exact(&mut frame[MESSAGE_HEADER_LENGTH..]) => {
            result.map_err(|_| SessionClientBridgeError::Transport)?;
        }
    }
    Ok(frame)
}

fn frame_type(frame: &[u8]) -> Result<u8, SessionClientBridgeError> {
    let header_bytes: [u8; MESSAGE_HEADER_LENGTH] = frame
        .get(..MESSAGE_HEADER_LENGTH)
        .ok_or(SessionClientBridgeError::Frame)?
        .try_into()
        .map_err(|_| SessionClientBridgeError::Frame)?;
    let header = MessageHeader::from(header_bytes);
    let body_len = usize::try_from(header.length).map_err(|_| SessionClientBridgeError::Frame)?;
    if body_len > MAX_LOGICAL_MESSAGE_BYTES as usize
        || frame.len() != MESSAGE_HEADER_LENGTH.saturating_add(body_len)
    {
        return Err(SessionClientBridgeError::Frame);
    }
    Ok(header.type_)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionClientBridgeError {
    Cancelled,
    Frame,
    Session(SessionError),
    Transport,
}

impl std::fmt::Display for SessionClientBridgeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Cancelled => "component-session-client-cancelled",
            Self::Frame => "component-session-client-frame-invalid",
            Self::Session(_) => "component-session-client-session-failed",
            Self::Transport => "component-session-client-transport-failed",
        })
    }
}

impl std::error::Error for SessionClientBridgeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use ttrpc::proto::MessageHeader;

    #[test]
    fn frame_type_rejects_trailing_bytes_and_oversize_bodies() {
        let mut frame = Vec::from(MessageHeader::new_response(7, 3));
        frame.extend_from_slice(b"abc");
        assert_eq!(frame_type(&frame), Ok(MESSAGE_TYPE_RESPONSE));
        frame.push(0);
        assert_eq!(frame_type(&frame), Err(SessionClientBridgeError::Frame));

        let header = MessageHeader {
            length: (MAX_LOGICAL_MESSAGE_BYTES + 1),
            stream_id: 7,
            type_: MESSAGE_TYPE_RESPONSE,
            flags: 0,
        };
        assert_eq!(
            frame_type(&Vec::from(header)),
            Err(SessionClientBridgeError::Frame)
        );
    }

    #[tokio::test]
    async fn read_frame_preserves_exact_ttrpc_boundaries() {
        let (mut writer, mut reader) = tokio::io::duplex(128);
        let first = {
            let mut frame = Vec::from(MessageHeader::new_response(1, 3));
            frame.extend_from_slice(b"one");
            frame
        };
        let second = {
            let mut frame = Vec::from(MessageHeader::new_response(3, 3));
            frame.extend_from_slice(b"two");
            frame
        };
        writer
            .write_all(&[&first[..], &second[..]].concat())
            .await
            .unwrap();
        let cancellation = Cancellation::new();
        assert_eq!(read_frame(&mut reader, &cancellation).await.unwrap(), first);
        assert_eq!(
            read_frame(&mut reader, &cancellation).await.unwrap(),
            second
        );
    }
}
