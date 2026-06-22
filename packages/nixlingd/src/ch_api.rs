//! Minimal Cloud Hypervisor HTTP-over-unix helper shared by lifecycle and
//! metrics code.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::Path;
use std::time::Duration;

use tokio::net::UnixStream;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChApiError {
    Unavailable(String),
    Timeout,
    ResponseTooLarge,
    MalformedResponse,
    Rejected(u16),
    InvalidJson(String),
}

impl ChApiError {
    pub fn bounded_label(&self) -> &'static str {
        match self {
            Self::Unavailable(_) => "api_unavailable",
            Self::Timeout => "timeout_exceeded",
            Self::ResponseTooLarge => "response_too_large",
            Self::MalformedResponse | Self::InvalidJson(_) => "malformed_response",
            Self::Rejected(_) => "api_rejected",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChVmInfo {
    pub state: Option<String>,
    pub vcpu_count: Option<u64>,
    pub memory_mib: Option<u64>,
}

pub async fn get_vm_info(socket: &Path, timeout: Duration) -> Result<ChVmInfo, ChApiError> {
    let body = request(socket, "GET", "/api/v1/vm.info", timeout).await?;
    parse_vm_info(&body)
}

pub async fn shutdown_vm(socket: &Path, timeout: Duration) -> Result<(), ChApiError> {
    request(socket, "PUT", "/api/v1/vm.shutdown", timeout)
        .await
        .map(|_| ())
}

pub fn blocking_get_json(
    socket: &Path,
    path: &str,
    timeout: Duration,
) -> Result<Vec<u8>, ChApiError> {
    let mut stream = StdUnixStream::connect(socket)
        .map_err(|err| ChApiError::Unavailable(classify_io_error(err)))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|err| ChApiError::Unavailable(err.to_string()))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|err| ChApiError::Unavailable(err.to_string()))?;
    let req = http_request("GET", path);
    stream
        .write_all(req.as_bytes())
        .map_err(|err| ChApiError::Unavailable(classify_io_error(err)))?;
    let raw = read_blocking_capped(&mut stream)?;
    split_http_body(&raw)
}

pub fn parse_vm_info(body: &[u8]) -> Result<ChVmInfo, ChApiError> {
    let v: serde_json::Value =
        serde_json::from_slice(body).map_err(|err| ChApiError::InvalidJson(err.to_string()))?;
    let state = v.get("state").and_then(|s| s.as_str()).map(str::to_owned);
    let vcpu_count = v
        .get("config")
        .and_then(|c| c.get("cpus"))
        .and_then(|c| c.get("boot_vcpus"))
        .and_then(|n| n.as_u64());
    let memory_mib = v
        .get("config")
        .and_then(|c| c.get("memory"))
        .and_then(|m| m.get("size"))
        .and_then(|n| n.as_u64());
    Ok(ChVmInfo {
        state,
        vcpu_count,
        memory_mib,
    })
}

async fn request(
    socket: &Path,
    method: &str,
    path: &str,
    timeout: Duration,
) -> Result<Vec<u8>, ChApiError> {
    let op = async {
        let stream = UnixStream::connect(socket)
            .await
            .map_err(|err| ChApiError::Unavailable(classify_io_error(err)))?;
        write_all(&stream, http_request(method, path).as_bytes()).await?;
        let raw = read_async_capped(&stream).await?;
        split_http_body(&raw)
    };
    tokio::time::timeout(timeout, op)
        .await
        .map_err(|_| ChApiError::Timeout)?
}

fn http_request(method: &str, path: &str) -> String {
    format!(
        "{method} {path} HTTP/1.0\r\nHost: localhost\r\nAccept: application/json\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    )
}

async fn write_all(stream: &UnixStream, mut bytes: &[u8]) -> Result<(), ChApiError> {
    while !bytes.is_empty() {
        stream
            .writable()
            .await
            .map_err(|err| ChApiError::Unavailable(classify_io_error(err)))?;
        match stream.try_write(bytes) {
            Ok(0) => return Err(ChApiError::Unavailable("short write".to_owned())),
            Ok(n) => bytes = &bytes[n..],
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(err) => return Err(ChApiError::Unavailable(classify_io_error(err))),
        }
    }
    Ok(())
}

async fn read_async_capped(stream: &UnixStream) -> Result<Vec<u8>, ChApiError> {
    let mut raw = Vec::with_capacity(2048);
    let mut buf = [0u8; 2048];
    loop {
        stream
            .readable()
            .await
            .map_err(|err| ChApiError::Unavailable(classify_io_error(err)))?;
        match stream.try_read(&mut buf) {
            Ok(0) => return Ok(raw),
            Ok(n) => {
                if raw.len().saturating_add(n) > MAX_RESPONSE_BYTES {
                    return Err(ChApiError::ResponseTooLarge);
                }
                raw.extend_from_slice(&buf[..n]);
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(err) => return Err(ChApiError::Unavailable(classify_io_error(err))),
        }
    }
}

fn read_blocking_capped(stream: &mut StdUnixStream) -> Result<Vec<u8>, ChApiError> {
    let mut raw = Vec::with_capacity(2048);
    let mut buf = [0u8; 2048];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => return Ok(raw),
            Ok(n) => {
                if raw.len().saturating_add(n) > MAX_RESPONSE_BYTES {
                    return Err(ChApiError::ResponseTooLarge);
                }
                raw.extend_from_slice(&buf[..n]);
            }
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Err(ChApiError::Timeout);
            }
            Err(err) => return Err(ChApiError::Unavailable(classify_io_error(err))),
        }
    }
}

pub(crate) fn split_http_body(raw: &[u8]) -> Result<Vec<u8>, ChApiError> {
    let status_end = raw
        .iter()
        .position(|b| *b == b'\n')
        .ok_or(ChApiError::MalformedResponse)?;
    let status_line = std::str::from_utf8(&raw[..status_end])
        .map_err(|_| ChApiError::MalformedResponse)?
        .trim_end();
    let mut parts = status_line.split_whitespace();
    let _version = parts.next().ok_or(ChApiError::MalformedResponse)?;
    let code: u16 = parts
        .next()
        .ok_or(ChApiError::MalformedResponse)?
        .parse()
        .map_err(|_| ChApiError::MalformedResponse)?;
    if !(200..300).contains(&code) {
        return Err(ChApiError::Rejected(code));
    }
    let sep = find_subslice(raw, b"\r\n\r\n")
        .map(|i| i + 4)
        .or_else(|| find_subslice(raw, b"\n\n").map(|i| i + 2))
        .ok_or(ChApiError::MalformedResponse)?;
    Ok(raw[sep..].to_vec())
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn classify_io_error(err: std::io::Error) -> String {
    match err.kind() {
        std::io::ErrorKind::NotFound => "not_found".to_owned(),
        std::io::ErrorKind::ConnectionRefused => "connection_refused".to_owned(),
        std::io::ErrorKind::ConnectionReset => "connection_reset".to_owned(),
        std::io::ErrorKind::BrokenPipe => "broken_pipe".to_owned(),
        std::io::ErrorKind::UnexpectedEof => "unexpected_eof".to_owned(),
        std::io::ErrorKind::TimedOut => "timeout".to_owned(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream as StdUnixStream;
    use std::thread;

    #[test]
    fn blocking_read_rejects_oversized_response() {
        let (mut reader, mut writer) = StdUnixStream::pair().expect("unix pair");
        let writer_thread = thread::spawn(move || {
            let _ = writer.write_all(b"HTTP/1.0 200 OK\r\n\r\n");
            let _ = writer.write_all(&vec![b'a'; MAX_RESPONSE_BYTES + 1]);
        });

        let error = read_blocking_capped(&mut reader).expect_err("oversized response rejected");
        writer_thread.join().expect("writer thread");
        assert_eq!(error, ChApiError::ResponseTooLarge);
    }

    #[test]
    fn vm_info_states_used_by_lifecycle_parse_cleanly() {
        for state in ["Running", "Paused", "Created", "Shutdown"] {
            let body = format!(r#"{{"state":"{state}","config":{{}}}}"#);
            let info = parse_vm_info(body.as_bytes()).expect("parse vm.info");
            assert_eq!(info.state.as_deref(), Some(state));
        }
    }
}
