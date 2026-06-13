//! Minimal length-prefixed binary codec primitives (no serde).
//!
//! All multi-byte integers are little-endian. Strings and byte blobs are
//! `u32` length-prefixed. Byte budgets are validated on decode so a corrupt
//! or hostile on-disk file can never drive an unbounded allocation.

/// Hard ceiling for any single length-prefixed field on decode. Generous
/// enough for the largest validated argv/env value, small enough to refuse a
/// corrupt length word.
pub const MAX_FIELD_BYTES: usize = 1 << 20;

/// Append-only byte writer.
#[derive(Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn put_u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    pub fn put_u32(&mut self, value: u32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    pub fn put_u64(&mut self, value: u64) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    pub fn put_i32(&mut self, value: i32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    pub fn put_bool(&mut self, value: bool) {
        self.buf.push(u8::from(value));
    }

    pub fn put_bytes(&mut self, bytes: &[u8]) {
        self.put_u32(bytes.len() as u32);
        self.buf.extend_from_slice(bytes);
    }

    pub fn put_str(&mut self, value: &str) {
        self.put_bytes(value.as_bytes());
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.buf
    }
}

/// Typed decode failure. Carries no payload bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    Truncated,
    FieldTooLong,
    InvalidUtf8,
    InvalidTag,
    TrailingBytes,
    BadMagic,
    BadVersion,
}

/// Cursor reader over a byte slice.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).ok_or(DecodeError::Truncated)?;
        if end > self.data.len() {
            return Err(DecodeError::Truncated);
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    pub fn get_u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    pub fn get_u32(&mut self) -> Result<u32, DecodeError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn get_u64(&mut self) -> Result<u64, DecodeError> {
        let bytes = self.take(8)?;
        let mut buf = [0u8; 8];
        buf.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(buf))
    }

    pub fn get_i32(&mut self) -> Result<i32, DecodeError> {
        let bytes = self.take(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn get_bool(&mut self) -> Result<bool, DecodeError> {
        match self.get_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::InvalidTag),
        }
    }

    pub fn get_bytes(&mut self) -> Result<Vec<u8>, DecodeError> {
        let len = self.get_u32()? as usize;
        if len > MAX_FIELD_BYTES {
            return Err(DecodeError::FieldTooLong);
        }
        Ok(self.take(len)?.to_vec())
    }

    pub fn get_str(&mut self) -> Result<String, DecodeError> {
        let bytes = self.get_bytes()?;
        String::from_utf8(bytes).map_err(|_| DecodeError::InvalidUtf8)
    }

    /// Ensure every byte was consumed; reject trailing garbage.
    pub fn finish(self) -> Result<(), DecodeError> {
        if self.pos == self.data.len() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_primitives() {
        let mut w = Writer::new();
        w.put_u8(7);
        w.put_u32(42);
        w.put_u64(1 << 40);
        w.put_i32(-9);
        w.put_bool(true);
        w.put_str("hello");
        w.put_bytes(&[1, 2, 3]);
        let bytes = w.into_vec();

        let mut r = Reader::new(&bytes);
        assert_eq!(r.get_u8().unwrap(), 7);
        assert_eq!(r.get_u32().unwrap(), 42);
        assert_eq!(r.get_u64().unwrap(), 1 << 40);
        assert_eq!(r.get_i32().unwrap(), -9);
        assert!(r.get_bool().unwrap());
        assert_eq!(r.get_str().unwrap(), "hello");
        assert_eq!(r.get_bytes().unwrap(), vec![1, 2, 3]);
        r.finish().unwrap();
    }

    #[test]
    fn rejects_truncated_and_oversized() {
        let mut r = Reader::new(&[0, 0]);
        assert_eq!(r.get_u32(), Err(DecodeError::Truncated));

        // A length word larger than MAX_FIELD_BYTES is refused before alloc.
        let mut w = Writer::new();
        w.put_u32((MAX_FIELD_BYTES + 1) as u32);
        let bytes = w.into_vec();
        let mut r = Reader::new(&bytes);
        assert_eq!(r.get_bytes(), Err(DecodeError::FieldTooLong));
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut w = Writer::new();
        w.put_u8(1);
        let mut bytes = w.into_vec();
        bytes.push(0xff);
        let mut r = Reader::new(&bytes);
        assert_eq!(r.get_u8().unwrap(), 1);
        assert_eq!(r.finish(), Err(DecodeError::TrailingBytes));
    }
}
