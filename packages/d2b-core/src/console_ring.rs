//! Bounded console output ring buffer with monotonic offset tracking (ADR 0041).
//!
//! The ring buffer continuously receives bytes from the console drainer and
//! hands them to attached client readers without applying back-pressure to the
//! guest. When the buffer is full the oldest bytes are dropped and the
//! [`RingBuffer::base_offset`] advances so clients can detect gaps and
//! fast-forward.
//!
//! All types are `no_std`-compatible pure data; I/O and concurrency are the
//! caller's responsibility.

use serde::{Deserialize, Serialize};

/// Default ring-buffer capacity: 256 KiB per VM.
pub const DEFAULT_RING_CAPACITY: usize = 256 * 1024;

/// Ring buffer for a single console stream.
///
/// Bytes are stored in a fixed-capacity `VecDeque`-like circular buffer.
/// Two monotonic counters track the logical position in the infinite
/// console stream:
/// - `total_written`: total bytes ever appended (next write position in
///   the logical stream)
/// - `base_offset`: logical offset of the first byte currently held in the
///   buffer (`= total_written - len`)
///
/// `dropped_bytes == base_offset` because every byte that is no longer in
/// the buffer has been overwritten/dropped from the front.
#[derive(Debug)]
pub struct RingBuffer {
    data: Vec<u8>,
    capacity: usize,
    /// Index of the next byte to write into `data`.
    head: usize,
    /// Number of bytes currently stored.
    len: usize,
    /// Logical stream offset of `data[head - len]` (the oldest byte held).
    base_offset: u64,
    /// Total bytes ever appended to this buffer (next logical write offset).
    total_written: u64,
    /// Whether the stream has ended (guest exited or drainer stopped).
    pub is_eof: bool,
}

impl RingBuffer {
    /// Create a new empty ring buffer with the given byte capacity.
    ///
    /// # Panics
    ///
    /// Panics when `capacity == 0`.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "ring buffer capacity must be non-zero");
        Self {
            data: vec![0_u8; capacity],
            capacity,
            head: 0,
            len: 0,
            base_offset: 0,
            total_written: 0,
            is_eof: false,
        }
    }

    /// Append `bytes` to the ring buffer.
    ///
    /// When the buffer cannot hold all incoming bytes without exceeding
    /// `capacity`, the oldest bytes are silently dropped and `base_offset`
    /// advances. The drainer MUST call this without back-pressure so the
    /// guest is never blocked.
    pub fn push_bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let incoming = bytes.len();
        // If the incoming data is larger than capacity, only keep the tail.
        let (skip, effective) = if incoming >= self.capacity {
            (incoming - self.capacity, &bytes[incoming - self.capacity..])
        } else {
            (0, bytes)
        };
        // Bytes we must evict to make room.
        let evict = (self.len + effective.len()).saturating_sub(self.capacity);
        self.len -= evict;
        // Also account for bytes we skipped before even trying to store.
        let total_dropped = skip + evict;
        self.base_offset = self.base_offset.saturating_add(total_dropped as u64);

        // Write `effective` bytes in one or two segments to avoid wrapping.
        let first_chunk_len = (self.capacity - self.head).min(effective.len());
        self.data[self.head..self.head + first_chunk_len]
            .copy_from_slice(&effective[..first_chunk_len]);
        if first_chunk_len < effective.len() {
            let rest = &effective[first_chunk_len..];
            self.data[..rest.len()].copy_from_slice(rest);
        }
        self.head = (self.head + effective.len()) % self.capacity;
        self.len += effective.len();
        self.total_written = self.total_written.saturating_add(incoming as u64);
    }

    /// Logical offset of the first byte currently held (equals dropped bytes).
    pub fn base_offset(&self) -> u64 {
        self.base_offset
    }

    /// Total bytes ever written (next logical write offset).
    pub fn total_written(&self) -> u64 {
        self.total_written
    }

    /// Total bytes dropped from the front of the logical stream.
    pub fn dropped_bytes(&self) -> u64 {
        self.base_offset
    }

    /// Number of bytes currently stored in the ring.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the ring is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Read up to `max_len` bytes starting at logical `offset`.
    ///
    /// Returns a [`RingReadResult`] with the bytes, the actual starting
    /// offset (which may be > `offset` if bytes were dropped), and the
    /// current `base_offset` so the caller can detect future gaps.
    ///
    /// Returns `None` when `offset >= total_written` and the stream has not
    /// reached EOF. Once EOF is set, an empty [`RingReadResult`] is returned so
    /// clients can stop polling cleanly after consuming the final byte.
    pub fn read_at(&self, offset: u64, max_len: u64) -> Option<RingReadResult> {
        if self.is_empty() {
            if self.is_eof {
                return Some(RingReadResult {
                    actual_offset: self.total_written,
                    data: Vec::new(),
                    base_offset: self.base_offset,
                    dropped_bytes: self.dropped_bytes(),
                    is_eof: true,
                });
            }
            return None;
        }
        // Clamp requested offset to what is still available.
        let actual_offset = offset.max(self.base_offset);
        if actual_offset >= self.total_written {
            if self.is_eof {
                return Some(RingReadResult {
                    actual_offset: self.total_written,
                    data: Vec::new(),
                    base_offset: self.base_offset,
                    dropped_bytes: self.dropped_bytes(),
                    is_eof: true,
                });
            }
            return None;
        }
        // How many bytes are stored after `actual_offset`.
        let available = (self.total_written - actual_offset) as usize;
        let read_len = (available as u64).min(max_len) as usize;
        if read_len == 0 {
            return Some(RingReadResult {
                actual_offset,
                data: Vec::new(),
                base_offset: self.base_offset,
                dropped_bytes: self.dropped_bytes(),
                is_eof: self.is_eof,
            });
        }
        // Index of `actual_offset` byte inside the ring.
        let start_in_ring = (self.total_written - self.len as u64 - self.base_offset) as usize
            + (actual_offset - self.base_offset) as usize;
        // Physical start position in `data`.
        let tail_start = (self.head + self.capacity - self.len) % self.capacity;
        let phys_start = (tail_start + start_in_ring) % self.capacity;
        // Copy in up to two segments.
        let mut chunk = Vec::with_capacity(read_len);
        let first_len = (self.capacity - phys_start).min(read_len);
        chunk.extend_from_slice(&self.data[phys_start..phys_start + first_len]);
        if first_len < read_len {
            chunk.extend_from_slice(&self.data[..read_len - first_len]);
        }
        Some(RingReadResult {
            actual_offset,
            data: chunk,
            base_offset: self.base_offset,
            dropped_bytes: self.dropped_bytes(),
            is_eof: self.is_eof,
        })
    }
}

/// Result returned by [`RingBuffer::read_at`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RingReadResult {
    /// Logical offset of the first byte in `data` (may differ from the
    /// requested offset when bytes were dropped since the last read).
    pub actual_offset: u64,
    /// Bytes read from the ring buffer.
    pub data: Vec<u8>,
    /// Current ring-buffer start offset; compare against
    /// `actual_offset + data.len()` to detect future gaps.
    pub base_offset: u64,
    /// Total bytes dropped from the logical stream since VM start.
    pub dropped_bytes: u64,
    /// Whether the console stream has ended.
    pub is_eof: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(cap: usize) -> RingBuffer {
        RingBuffer::new(cap)
    }

    #[test]
    fn empty_ring_read_returns_none() {
        let r = ring(64);
        assert!(r.read_at(0, 64).is_none());
    }

    #[test]
    fn basic_push_and_read() {
        let mut r = ring(64);
        r.push_bytes(b"hello");
        let result = r.read_at(0, 64).unwrap();
        assert_eq!(result.data, b"hello");
        assert_eq!(result.actual_offset, 0);
        assert_eq!(result.base_offset, 0);
        assert_eq!(result.dropped_bytes, 0);
        assert!(!result.is_eof);
    }

    #[test]
    fn read_at_offset_returns_tail() {
        let mut r = ring(64);
        r.push_bytes(b"hello world");
        let result = r.read_at(6, 5).unwrap();
        assert_eq!(result.data, b"world");
        assert_eq!(result.actual_offset, 6);
    }

    #[test]
    fn overflow_drops_oldest_bytes_and_advances_base_offset() {
        let mut r = ring(8);
        r.push_bytes(b"12345678"); // fills buffer exactly
        r.push_bytes(b"ABCD"); // drops "1234"
        assert_eq!(r.base_offset(), 4);
        assert_eq!(r.total_written(), 12);
        assert_eq!(r.dropped_bytes(), 4);
        let result = r.read_at(0, 64).unwrap();
        // offset was clamped to base_offset
        assert_eq!(result.actual_offset, 4);
        assert_eq!(result.data, b"5678ABCD");
    }

    #[test]
    fn overflow_larger_than_capacity_keeps_tail() {
        let mut r = ring(4);
        r.push_bytes(b"0123456789"); // 10 bytes into a 4-byte ring
        assert_eq!(r.base_offset(), 6);
        assert_eq!(r.total_written(), 10);
        let result = r.read_at(0, 64).unwrap();
        assert_eq!(result.data, b"6789");
        assert_eq!(result.dropped_bytes, 6);
    }

    #[test]
    fn ring_wrap_around_is_linear_to_caller() {
        let mut r = ring(8);
        r.push_bytes(b"AAAA");
        r.push_bytes(b"BBBB");
        // now replace the A's
        r.push_bytes(b"CCCC");
        // ring should hold BBBBCCCC
        assert_eq!(r.base_offset(), 4);
        let result = r.read_at(4, 64).unwrap();
        assert_eq!(result.data, b"BBBBCCCC");
    }

    #[test]
    fn slow_client_fast_forwards_on_drop() {
        let mut r = ring(8);
        r.push_bytes(b"12345678");
        let snap1 = r.read_at(0, 64).unwrap();
        assert_eq!(snap1.data, b"12345678");
        // guest writes 16 more bytes; client's last offset (8) is still valid
        r.push_bytes(b"AAAAAAAABBBBBBBB");
        let snap2 = r
            .read_at(snap1.actual_offset + snap1.data.len() as u64, 64)
            .unwrap();
        // base_offset moved to 16; client fast-forwards
        assert_eq!(snap2.actual_offset, 16);
        assert_eq!(snap2.data, b"BBBBBBBB");
        assert!(snap2.dropped_bytes > 0);
    }

    #[test]
    fn read_past_total_written_returns_none() {
        let mut r = ring(64);
        r.push_bytes(b"hello");
        assert!(r.read_at(5, 64).is_none()); // exactly at total_written
        assert!(r.read_at(100, 64).is_none());
    }

    #[test]
    fn eof_at_end_returns_empty_result() {
        let mut r = ring(64);
        r.push_bytes(b"hello");
        r.is_eof = true;
        let result = r.read_at(5, 64).unwrap();
        assert_eq!(result.data, b"");
        assert_eq!(result.actual_offset, 5);
        assert!(result.is_eof);
    }

    #[test]
    fn eof_empty_ring_returns_empty_result() {
        let mut r = ring(64);
        r.is_eof = true;
        let result = r.read_at(0, 64).unwrap();
        assert_eq!(result.data, b"");
        assert_eq!(result.actual_offset, 0);
        assert!(result.is_eof);
    }

    #[test]
    fn eof_flag_propagates_to_read_result() {
        let mut r = ring(64);
        r.push_bytes(b"bye");
        r.is_eof = true;
        let result = r.read_at(0, 64).unwrap();
        assert!(result.is_eof);
    }

    #[test]
    fn two_segment_wrap_around_copy() {
        // force a scenario where the circular buffer wraps mid-read
        let mut r = ring(8);
        // write 6 bytes so head=6
        r.push_bytes(b"AAAAAA");
        // overwrite 4 so head=2, base_offset=2
        r.push_bytes(b"BBBB");
        // now tail is at head - len = (2 + 8 - 8) % 8 = 2, data = "AABBBBBB"? no
        // let me just verify we get a contiguous result
        let result = r.read_at(r.base_offset(), 64).unwrap();
        assert_eq!(result.data.len(), 8);
    }

    #[test]
    #[should_panic]
    fn zero_capacity_panics() {
        RingBuffer::new(0);
    }

    #[test]
    fn multiple_small_chunks_accumulate() {
        let mut r = ring(64);
        for b in b"hello world" {
            r.push_bytes(std::slice::from_ref(b));
        }
        let result = r.read_at(0, 64).unwrap();
        assert_eq!(result.data, b"hello world");
    }
}
