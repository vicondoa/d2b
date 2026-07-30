//! Incremental UTF-8 decoding across read boundaries.
//!
//! The PTY hands us arbitrary byte chunks. A multi-byte character can be split
//! across two `read(2)` calls, so decoding each chunk independently with
//! `String::from_utf8_lossy` turns that character into replacement characters.
//! `ht` has exactly this bug (`src/session.rs` calls `from_utf8_lossy` per
//! chunk); this decoder carries the partial tail forward instead.

/// Longest possible UTF-8 encoding of a single scalar value.
const MAX_UTF8_LEN: usize = 4;

/// A streaming UTF-8 decoder that buffers an incomplete trailing sequence.
#[derive(Debug, Default)]
pub struct Utf8Decoder {
    /// Bytes of an incomplete sequence carried over from the previous feed.
    partial: Vec<u8>,
}

impl Utf8Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode `input`, returning all characters that are now complete.
    ///
    /// Any trailing bytes that form the prefix of a still-incomplete sequence
    /// are retained and consumed by the next call. Genuinely invalid bytes are
    /// replaced with U+FFFD, matching what a real terminal does.
    pub fn feed(&mut self, input: &[u8]) -> String {
        let buf: Vec<u8> = if self.partial.is_empty() {
            input.to_vec()
        } else {
            let mut joined = std::mem::take(&mut self.partial);
            joined.extend_from_slice(input);
            joined
        };

        let mut out = String::with_capacity(buf.len());
        let mut idx = 0usize;

        while idx < buf.len() {
            match std::str::from_utf8(&buf[idx..]) {
                Ok(rest) => {
                    out.push_str(rest);
                    idx = buf.len();
                }
                Err(err) => {
                    let valid = err.valid_up_to();
                    if valid > 0 {
                        // `valid_up_to` is a validated boundary by contract.
                        out.push_str(&String::from_utf8_lossy(&buf[idx..idx + valid]));
                        idx += valid;
                    }

                    match err.error_len() {
                        // Genuinely invalid byte(s): emit U+FFFD and skip them.
                        Some(bad) => {
                            out.push('\u{FFFD}');
                            idx += bad;
                        }
                        // Incomplete tail: stash it for the next feed.
                        None => {
                            let tail = &buf[idx..];
                            if tail.len() < MAX_UTF8_LEN {
                                self.partial.extend_from_slice(tail);
                            } else {
                                // Longer than any legal sequence, so it can
                                // never complete. Do not stash it forever.
                                out.push('\u{FFFD}');
                            }
                            idx = buf.len();
                        }
                    }
                }
            }
        }

        out
    }

    /// Flush any retained incomplete sequence as a replacement character.
    ///
    /// Called at end of stream so a truncated final character is not silently
    /// dropped.
    pub fn flush(&mut self) -> String {
        if self.partial.is_empty() {
            String::new()
        } else {
            self.partial.clear();
            "\u{FFFD}".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Utf8Decoder;

    #[test]
    fn ascii_passes_through() {
        let mut d = Utf8Decoder::new();
        assert_eq!(d.feed(b"hello"), "hello");
    }

    #[test]
    fn multibyte_split_across_two_feeds_is_not_corrupted() {
        // U+00E9 LATIN SMALL LETTER E WITH ACUTE is 0xC3 0xA9.
        let mut d = Utf8Decoder::new();
        assert_eq!(d.feed(&[0xC3]), "");
        assert_eq!(d.feed(&[0xA9]), "\u{e9}");
    }

    #[test]
    fn four_byte_split_across_four_feeds() {
        // U+1F600 GRINNING FACE is 0xF0 0x9F 0x98 0x80.
        let mut d = Utf8Decoder::new();
        assert_eq!(d.feed(&[0xF0]), "");
        assert_eq!(d.feed(&[0x9F]), "");
        assert_eq!(d.feed(&[0x98]), "");
        assert_eq!(d.feed(&[0x80]), "\u{1F600}");
    }

    #[test]
    fn text_around_a_split_char_is_preserved() {
        let mut d = Utf8Decoder::new();
        assert_eq!(d.feed(b"ab\xE6\x97"), "ab");
        assert_eq!(d.feed(b"\xA5cd"), "\u{65e5}cd");
    }

    #[test]
    fn invalid_byte_becomes_replacement_and_stream_continues() {
        let mut d = Utf8Decoder::new();
        assert_eq!(d.feed(&[b'a', 0xFF, b'b']), "a\u{FFFD}b");
    }

    #[test]
    fn flush_emits_replacement_for_truncated_tail() {
        let mut d = Utf8Decoder::new();
        assert_eq!(d.feed(&[0xC3]), "");
        assert_eq!(d.flush(), "\u{FFFD}");
        // Flushing twice must not keep emitting.
        assert_eq!(d.flush(), "");
    }

    #[test]
    fn flush_is_empty_when_aligned() {
        let mut d = Utf8Decoder::new();
        assert_eq!(d.feed(b"ok"), "ok");
        assert_eq!(d.flush(), "");
    }
}
