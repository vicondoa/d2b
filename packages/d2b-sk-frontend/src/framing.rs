//! CTAPHID framing over a byte-stream transport.
//!
//! Every CTAPHID HID report is exactly 64 bytes. The framing layer wraps each
//! report in a 4-byte little-endian length prefix so the stream boundary is
//! unambiguous even when a reconnect races a partial write.
//!
//! Wire format per frame:
//!   [u32 LE: length (must equal CTAPHID_REPORT_LEN)] [length bytes: payload]
//!
//! A received length field that does not equal `CTAPHID_REPORT_LEN` is a
//! protocol error — the connection must be dropped and restarted.

use std::io;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Fixed size of a CTAPHID HID report (input or output).
pub const CTAPHID_REPORT_LEN: usize = 64;

/// Read one framed CTAPHID report from `reader`.
///
/// Returns `None` on clean EOF (peer closed the connection). Returns
/// `Err` on I/O error or protocol violation (unexpected length field).
pub async fn read_frame<R>(reader: &mut R) -> io::Result<Option<[u8; CTAPHID_REPORT_LEN]>>
where
    R: AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len != CTAPHID_REPORT_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("framing: expected length {CTAPHID_REPORT_LEN}, got {len}"),
        ));
    }
    let mut payload = [0u8; CTAPHID_REPORT_LEN];
    reader.read_exact(&mut payload).await?;
    Ok(Some(payload))
}

/// Write one framed CTAPHID report to `writer`.
pub async fn write_frame<W>(writer: &mut W, data: &[u8; CTAPHID_REPORT_LEN]) -> io::Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let len = (CTAPHID_REPORT_LEN as u32).to_le_bytes();
    let mut buf = [0u8; 4 + CTAPHID_REPORT_LEN];
    buf[..4].copy_from_slice(&len);
    buf[4..].copy_from_slice(data);
    writer.write_all(&buf).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufWriter;

    #[tokio::test]
    async fn roundtrip_frame() {
        let mut report = [0u8; CTAPHID_REPORT_LEN];
        report[0] = 0xff;
        report[1] = 0xff;
        report[2] = 0xff;
        report[3] = 0xff;
        report[4] = 0x86; // CTAPHID_INIT
        report[5] = 0x00;
        report[6] = 0x08;
        report[7] = 0x01;
        report[8] = 0x02;
        report[9] = 0x03;
        report[10] = 0x04;
        report[11] = 0x05;
        report[12] = 0x06;
        report[13] = 0x07;
        report[14] = 0x08;

        let mut buf: Vec<u8> = Vec::new();
        {
            let mut writer = BufWriter::new(&mut buf);
            write_frame(&mut writer, &report).await.unwrap();
            writer.flush().await.unwrap();
        }
        assert_eq!(buf.len(), 4 + CTAPHID_REPORT_LEN);
        assert_eq!(&buf[..4], &(CTAPHID_REPORT_LEN as u32).to_le_bytes());

        let mut cursor = tokio::io::BufReader::new(buf.as_slice());
        let got = read_frame(&mut cursor).await.unwrap().unwrap();
        assert_eq!(got, report);
    }

    #[tokio::test]
    async fn reject_wrong_length() {
        let wrong_len: u32 = 63;
        let mut buf = Vec::new();
        buf.extend_from_slice(&wrong_len.to_le_bytes());
        buf.extend_from_slice(&[0u8; 63]);

        let mut cursor = tokio::io::BufReader::new(buf.as_slice());
        let result = read_frame(&mut cursor).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn eof_returns_none() {
        let buf: &[u8] = &[];
        let mut cursor = tokio::io::BufReader::new(buf);
        let result = read_frame(&mut cursor).await.unwrap();
        assert!(result.is_none());
    }
}
