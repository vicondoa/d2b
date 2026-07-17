use std::{collections::HashMap, sync::Arc};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use ttrpc::{
    r#async::Service,
    proto::{MESSAGE_HEADER_LENGTH, MessageHeader},
};

use crate::ComponentSessionDriver;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionServerError {
    Service,
    Session,
    Transport,
    Frame,
}

impl std::fmt::Display for SessionServerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Service => "component-session-service-failed",
            Self::Session => "component-session-driver-failed",
            Self::Transport => "component-session-service-transport-failed",
            Self::Frame => "component-session-ttrpc-frame-invalid",
        })
    }
}

impl std::error::Error for SessionServerError {}

pub async fn serve_ttrpc_services(
    driver: Arc<dyn ComponentSessionDriver>,
    services: HashMap<String, Service>,
) -> Result<(), SessionServerError> {
    if services.is_empty() {
        return Err(SessionServerError::Service);
    }
    let capacity =
        d2b_contracts::v2_component_session::LimitProfile::local_default().logical_ttrpc_bytes;
    let capacity = usize::try_from(capacity).map_err(|_| SessionServerError::Transport)?;
    let (server_transport, bridge_transport) = tokio::io::duplex(capacity);
    let listener =
        ttrpc::r#async::transport::Listener::new(futures_util::stream::once(async move {
            Ok::<_, std::io::Error>(server_transport)
        }));
    let mut server = ttrpc::r#async::Server::new()
        .add_listener(listener)
        .register_service(services);
    server
        .start()
        .await
        .map_err(|_| SessionServerError::Service)?;

    let (mut bridge_reader, mut bridge_writer) = tokio::io::split(bridge_transport);
    let receive_driver = Arc::clone(&driver);
    let receive = async move {
        loop {
            let frame = receive_driver
                .receive_ttrpc()
                .await
                .map_err(|_| SessionServerError::Session)?;
            validate_frame(&frame)?;
            bridge_writer
                .write_all(&frame)
                .await
                .map_err(|_| SessionServerError::Transport)?;
            bridge_writer
                .flush()
                .await
                .map_err(|_| SessionServerError::Transport)?;
        }
    };
    let send = async move {
        loop {
            let mut header_bytes = [0_u8; MESSAGE_HEADER_LENGTH];
            bridge_reader
                .read_exact(&mut header_bytes)
                .await
                .map_err(|_| SessionServerError::Transport)?;
            let header = MessageHeader::from(header_bytes);
            let body_len = usize::try_from(header.length).map_err(|_| SessionServerError::Frame)?;
            if body_len > d2b_contracts::v2_component_session::MAX_LOGICAL_MESSAGE_BYTES as usize {
                return Err(SessionServerError::Frame);
            }
            let mut frame = header_bytes.to_vec();
            frame.resize(
                MESSAGE_HEADER_LENGTH
                    .checked_add(body_len)
                    .ok_or(SessionServerError::Frame)?,
                0,
            );
            bridge_reader
                .read_exact(&mut frame[MESSAGE_HEADER_LENGTH..])
                .await
                .map_err(|_| SessionServerError::Transport)?;
            driver
                .send_ttrpc(frame)
                .await
                .map_err(|_| SessionServerError::Session)?;
        }
    };
    let result = tokio::select! {
        result = receive => result,
        result = send => result,
    };
    server.disconnect().await;
    result
}

fn validate_frame(frame: &[u8]) -> Result<(), SessionServerError> {
    let header_bytes: [u8; MESSAGE_HEADER_LENGTH] = frame
        .get(..MESSAGE_HEADER_LENGTH)
        .ok_or(SessionServerError::Frame)?
        .try_into()
        .map_err(|_| SessionServerError::Frame)?;
    let header = MessageHeader::from(header_bytes);
    let body_len = usize::try_from(header.length).map_err(|_| SessionServerError::Frame)?;
    if body_len > d2b_contracts::v2_component_session::MAX_LOGICAL_MESSAGE_BYTES as usize
        || frame.len() != MESSAGE_HEADER_LENGTH.saturating_add(body_len)
    {
        return Err(SessionServerError::Frame);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_validation_is_exact_and_bounded() {
        let header = MessageHeader {
            length: 3,
            stream_id: 7,
            type_: 1,
            flags: 0,
        };
        let mut frame = Vec::from(header);
        frame.extend_from_slice(b"abc");
        assert_eq!(validate_frame(&frame), Ok(()));
        frame.push(0);
        assert_eq!(validate_frame(&frame), Err(SessionServerError::Frame));
    }
}
