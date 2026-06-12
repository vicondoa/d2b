//! File-backed, drop-oldest output ring with monotonic logical offsets.
//!
//! The semantics mirror guestd's in-memory `OutputRing` exactly (a parity test
//! asserts this), but the bytes live in a fixed-size circular file on tmpfs so
//! the runner (writer) and guestd (reader) can be different processes and so
//! the bytes survive a guestd restart for re-adoption.
//!
//! Layout per stream:
//! - `<stream>`      — a circular data file whose physical size never exceeds
//!   `cap`. The byte at logical offset `o` lives at physical `o % cap`.
//! - `<stream>.meta` — a small [`StreamMeta`] sidecar written atomically
//!   (temp -> fsync -> rename -> dir fsync).
//!
//! Durability ordering (the crux of the cross-process protocol):
//! - When an append drops oldest bytes (advancing `start_offset`), the writer
//!   persists the advanced `start_offset` BEFORE physically overwriting the
//!   cells, so a concurrent reader observes the start advance and expires the
//!   offset rather than reading a torn region.
//! - Data is always flushed (`sync_data`) BEFORE the sidecar publishes a larger
//!   `end_offset`, so a reader never sees offsets ahead of durable bytes.
//!
//! This module is dependency-pure (std only); symlink-safe opens use
//! `O_NOFOLLOW` via `OpenOptionsExt::custom_flags`.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::atomicio::{atomic_write, open_read_nofollow, read_file_nofollow, O_NOFOLLOW};
use crate::codec::{DecodeError, Reader, Writer};

const META_MAGIC: u32 = 0x4e4c_534d; // "NLSM"
const META_VERSION: u32 = 1;

/// Bounded retries for the reader's torn-read detection.
const READ_RETRIES: u32 = 8;

/// The sidecar contents: the durable ring bookkeeping for one stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamMeta {
    pub cap: u64,
    /// Logical offset of the oldest retained byte (== `dropped_bytes`).
    pub start_offset: u64,
    /// Logical offset one past the newest byte (== total bytes ever written).
    pub end_offset: u64,
    /// Count of bytes dropped off the front (always equal to `start_offset`).
    pub dropped_bytes: u64,
    /// True once any byte has been dropped.
    pub truncated: bool,
    /// True once the stream has been closed by the writer.
    pub eof: bool,
    /// True when guestd marked the stream lost (runner/unit vanished without a
    /// clean EOF). ORs into a chunk's `truncated`.
    pub lost: bool,
}

impl StreamMeta {
    fn new(cap: u64) -> Self {
        Self {
            cap,
            start_offset: 0,
            end_offset: 0,
            dropped_bytes: 0,
            truncated: false,
            eof: false,
            lost: false,
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.put_u32(META_MAGIC);
        w.put_u32(META_VERSION);
        w.put_u64(self.cap);
        w.put_u64(self.start_offset);
        w.put_u64(self.end_offset);
        w.put_u64(self.dropped_bytes);
        w.put_bool(self.truncated);
        w.put_bool(self.eof);
        w.put_bool(self.lost);
        w.into_vec()
    }

    fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(bytes);
        if r.get_u32()? != META_MAGIC {
            return Err(DecodeError::BadMagic);
        }
        if r.get_u32()? != META_VERSION {
            return Err(DecodeError::BadVersion);
        }
        let cap = r.get_u64()?;
        let start_offset = r.get_u64()?;
        let end_offset = r.get_u64()?;
        let dropped_bytes = r.get_u64()?;
        let truncated = r.get_bool()?;
        let eof = r.get_bool()?;
        let lost = r.get_bool()?;
        r.finish()?;
        Ok(Self {
            cap,
            start_offset,
            end_offset,
            dropped_bytes,
            truncated,
            eof,
            lost,
        })
    }
}

/// Result of a ring read. Field-for-field compatible with guestd's `RingChunk`.
#[derive(Clone, PartialEq, Eq)]
pub struct RingChunk {
    pub data: Vec<u8>,
    pub start_offset: u64,
    pub end_offset: u64,
    pub next_offset: u64,
    pub dropped_bytes: u64,
    pub truncated: bool,
    pub eof: bool,
}

// Redacted Debug: the captured `data` bytes are NEVER printed (they would leak
// child stdout/stderr into any log/panic/test-failure message). Only the
// bounded length and metadata are surfaced.
impl std::fmt::Debug for RingChunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RingChunk")
            .field("data_len", &self.data.len())
            .field("start_offset", &self.start_offset)
            .field("end_offset", &self.end_offset)
            .field("next_offset", &self.next_offset)
            .field("dropped_bytes", &self.dropped_bytes)
            .field("truncated", &self.truncated)
            .field("eof", &self.eof)
            .finish()
    }
}

/// Typed FileRing failure. Carries no payload bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRingError {
    Io(io::ErrorKind),
    Decode(DecodeError),
    OffsetExpired,
    OffsetInFuture,
    CapMismatch,
    /// The sidecar metadata is internally inconsistent or inconsistent with the
    /// data file (corrupt/tampered pair); never serve bytes from it.
    Corrupt,
    /// The reader could not obtain a stable (un-torn) read within the retry
    /// budget; the caller should treat this as an expired offset.
    Busy,
}

impl From<io::Error> for FileRingError {
    fn from(error: io::Error) -> Self {
        FileRingError::Io(error.kind())
    }
}

impl From<DecodeError> for FileRingError {
    fn from(error: DecodeError) -> Self {
        FileRingError::Decode(error)
    }
}

fn open_nofollow_write(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        // Privileged captured output; create 0600 explicitly, never umask.
        .mode(crate::atomicio::FILE_MODE_0600)
        .custom_flags(O_NOFOLLOW)
        .open(path)
}

/// Atomically replace the sidecar at `sidecar_path` with `meta`.
fn persist_meta(sidecar_path: &Path, meta: &StreamMeta) -> io::Result<()> {
    atomic_write(sidecar_path, &meta.encode())
}

fn read_meta(sidecar_path: &Path) -> Result<StreamMeta, FileRingError> {
    let bytes = read_file_nofollow(sidecar_path)?;
    StreamMeta::decode(&bytes).map_err(FileRingError::from)
}

/// Validate a decoded sidecar before any byte is served from the ring. Rejects
/// a corrupt/tampered pair (offsets ahead of data, impossible cap,
/// dropped/start drift) so the reader never indexes an invalid ring region.
fn validate_meta(meta: &StreamMeta, data_len: u64) -> Result<(), FileRingError> {
    // cap must be positive and within the largest cap the framework ever
    // reserves (a per-stream retained-log cap). Test rings use much smaller
    // caps, all well under this ceiling.
    if meta.cap == 0 || meta.cap > crate::DETACHED_STREAM_LOG_BYTES {
        return Err(FileRingError::Corrupt);
    }
    // Monotonic offsets: start never exceeds end.
    if meta.start_offset > meta.end_offset {
        return Err(FileRingError::Corrupt);
    }
    // The retained span can never exceed the ring capacity.
    if meta.end_offset - meta.start_offset > meta.cap {
        return Err(FileRingError::Corrupt);
    }
    // dropped_bytes is, by construction, exactly the advanced start offset.
    if meta.dropped_bytes != meta.start_offset {
        return Err(FileRingError::Corrupt);
    }
    // The physical data file must cover every cell we could index: the live
    // region is `min(end_offset, cap)` bytes (the file only grows to `cap`).
    let required_physical = meta.end_offset.min(meta.cap);
    if data_len < required_physical {
        return Err(FileRingError::Corrupt);
    }
    Ok(())
}

/// The writer side of a stream's ring (owned by the runner).
pub struct FileRing {
    data: File,
    data_path: PathBuf,
    sidecar_path: PathBuf,
    meta: StreamMeta,
}

impl FileRing {
    /// Create (or reopen) a stream's data + sidecar files with capacity `cap`.
    /// A fresh stream starts empty; reopening an existing stream restores its
    /// persisted meta (used by the runner only for its own freshly created
    /// slot, never to adopt a foreign one).
    pub fn create(data_path: &Path, sidecar_path: &Path, cap: u64) -> Result<Self, FileRingError> {
        assert!(cap > 0, "ring capacity must be positive");
        let data = open_nofollow_write(data_path)?;
        let meta = StreamMeta::new(cap);
        persist_meta(sidecar_path, &meta)?;
        Ok(Self {
            data,
            data_path: data_path.to_path_buf(),
            sidecar_path: sidecar_path.to_path_buf(),
            meta,
        })
    }

    pub fn meta(&self) -> StreamMeta {
        self.meta
    }

    pub fn data_path(&self) -> &Path {
        &self.data_path
    }

    pub fn sidecar_path(&self) -> &Path {
        &self.sidecar_path
    }

    /// Append `bytes`, dropping oldest bytes beyond `cap`. Persists the sidecar
    /// durably (start-before-overwrite, data-before-end).
    pub fn append(&mut self, bytes: &[u8]) -> Result<(), FileRingError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let cap = self.meta.cap;
        let incoming = bytes.len() as u64;
        let new_end = self.meta.end_offset.saturating_add(incoming);
        let new_start = new_end.saturating_sub(cap);

        // Phase 1: if this append drops oldest bytes, publish the advanced
        // start (without advancing end) BEFORE physically overwriting cells, so
        // a concurrent reader expires the offset instead of reading a torn
        // region. `end` is left unchanged here.
        if new_start > self.meta.start_offset {
            let pre = StreamMeta {
                start_offset: new_start.min(self.meta.end_offset),
                dropped_bytes: new_start.min(self.meta.end_offset),
                truncated: true,
                ..self.meta
            };
            persist_meta(&self.sidecar_path, &pre)?;
            self.meta = pre;
        }

        // Only the surviving tail of the incoming bytes is worth writing.
        let write_from = new_start.max(self.meta.end_offset);
        let skip = (write_from - self.meta.end_offset) as usize;
        let payload = &bytes[skip..];
        self.write_circular(write_from, payload)?;

        // Phase 3: flush data before publishing the larger end.
        self.data.sync_data()?;

        // Phase 4: publish the final {start, end} snapshot.
        self.meta.end_offset = new_end;
        if new_start > self.meta.start_offset {
            self.meta.start_offset = new_start;
            self.meta.dropped_bytes = new_start;
            self.meta.truncated = true;
        }
        persist_meta(&self.sidecar_path, &self.meta)?;
        Ok(())
    }

    /// Mark the stream closed (clean EOF).
    pub fn mark_eof(&mut self) -> Result<(), FileRingError> {
        if self.meta.eof {
            return Ok(());
        }
        self.meta.eof = true;
        persist_meta(&self.sidecar_path, &self.meta)?;
        Ok(())
    }

    /// Mark the stream lost (writer/unit vanished). Sets `lost` and `eof` so
    /// readers terminate; `lost` ORs into chunk `truncated`.
    pub fn mark_lost(&mut self) -> Result<(), FileRingError> {
        if self.meta.lost && self.meta.eof {
            return Ok(());
        }
        self.meta.lost = true;
        self.meta.eof = true;
        persist_meta(&self.sidecar_path, &self.meta)?;
        Ok(())
    }

    fn write_circular(&self, logical_start: u64, data: &[u8]) -> Result<(), FileRingError> {
        if data.is_empty() {
            return Ok(());
        }
        let cap = self.meta.cap as usize;
        let mut pos = (logical_start % self.meta.cap) as usize;
        let mut rem = data;
        while !rem.is_empty() {
            let space = cap - pos;
            let n = rem.len().min(space);
            self.data.write_all_at(&rem[..n], pos as u64)?;
            pos = (pos + n) % cap;
            rem = &rem[n..];
        }
        Ok(())
    }
}

/// The reader side of a stream's ring (owned by guestd).
pub struct FileRingReader {
    data: File,
    sidecar_path: PathBuf,
}

impl FileRingReader {
    pub fn open(data_path: &Path, sidecar_path: &Path) -> Result<Self, FileRingError> {
        let data = open_read_nofollow(data_path)?;
        Ok(Self {
            data,
            sidecar_path: sidecar_path.to_path_buf(),
        })
    }

    /// Read the sidecar and validate it is internally consistent and consistent
    /// with the actual data-file length before any byte is served. A corrupt or
    /// tampered sidecar (offsets ahead of data, impossible cap, dropped/start
    /// drift) is rejected as [`FileRingError::Corrupt`] rather than used to read
    /// from an invalid ring region.
    fn read_validated_meta(&self) -> Result<StreamMeta, FileRingError> {
        let meta = read_meta(&self.sidecar_path)?;
        let data_len = self.data.metadata().map(|m| m.len()).map_err(FileRingError::from)?;
        validate_meta(&meta, data_len)?;
        Ok(meta)
    }

    pub fn meta(&self) -> Result<StreamMeta, FileRingError> {
        self.read_validated_meta()
    }

    /// Read up to `max_len` bytes starting at logical `offset`. Mirrors
    /// `OutputRing::read`: `OffsetExpired` if `offset < start_offset`,
    /// `OffsetInFuture` if `offset > end_offset`.
    pub fn read(&self, offset: u64, max_len: u64) -> Result<RingChunk, FileRingError> {
        let mut last_err = FileRingError::Busy;
        for _ in 0..READ_RETRIES {
            let meta = self.read_validated_meta()?;
            if offset < meta.start_offset {
                return Err(FileRingError::OffsetExpired);
            }
            if offset > meta.end_offset {
                return Err(FileRingError::OffsetInFuture);
            }
            let available = meta.end_offset - offset;
            let take = available.min(max_len);
            let data = match self.read_circular(offset, take, meta.cap) {
                Ok(data) => data,
                Err(err) => {
                    last_err = err;
                    continue;
                }
            };

            // Torn-read detection: if the writer advanced start past our offset
            // during the read, the physical region may have been overwritten.
            let after = self.read_validated_meta()?;
            if after.start_offset > offset {
                last_err = FileRingError::OffsetExpired;
                continue;
            }

            let next_offset = offset.saturating_add(take);
            return Ok(RingChunk {
                data,
                start_offset: meta.start_offset,
                end_offset: meta.end_offset,
                next_offset,
                dropped_bytes: meta.dropped_bytes,
                truncated: meta.truncated || meta.lost,
                eof: meta.eof && next_offset >= meta.end_offset,
            });
        }
        Err(last_err)
    }

    fn read_circular(&self, offset: u64, take: u64, cap: u64) -> Result<Vec<u8>, FileRingError> {
        if take == 0 {
            return Ok(Vec::new());
        }
        if cap == 0 {
            return Err(FileRingError::CapMismatch);
        }
        let cap_usize = cap as usize;
        let mut out = vec![0u8; take as usize];
        let mut pos = (offset % cap) as usize;
        let mut written = 0usize;
        while written < out.len() {
            let space = cap_usize - pos;
            let n = (out.len() - written).min(space);
            self.data.read_exact_at(&mut out[written..written + n], pos as u64)?;
            pos = (pos + n) % cap_usize;
            written += n;
        }
        Ok(out)
    }
}

/// Mark a stream lost given only its sidecar path (guestd live reconciliation
/// when no [`FileRing`] writer is held). Idempotent.
pub fn mark_stream_lost(sidecar_path: &Path) -> Result<(), FileRingError> {
    let mut meta = read_meta(sidecar_path)?;
    if meta.lost && meta.eof {
        return Ok(());
    }
    meta.lost = true;
    meta.eof = true;
    persist_meta(sidecar_path, &meta)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Local copy of guestd's OutputRing semantics for the parity assertion.
    struct ModelRing {
        start_offset: u64,
        data: std::collections::VecDeque<u8>,
        cap: usize,
        dropped_bytes: u64,
        truncated: bool,
        eof: bool,
    }

    impl ModelRing {
        fn new(cap: usize) -> Self {
            Self {
                start_offset: 0,
                data: std::collections::VecDeque::new(),
                cap,
                dropped_bytes: 0,
                truncated: false,
                eof: false,
            }
        }

        fn end_offset(&self) -> u64 {
            self.start_offset.saturating_add(self.data.len() as u64)
        }

        fn append(&mut self, bytes: &[u8]) {
            self.data.extend(bytes.iter().copied());
            while self.data.len() > self.cap {
                if self.data.pop_front().is_some() {
                    self.start_offset = self.start_offset.saturating_add(1);
                    self.dropped_bytes = self.dropped_bytes.saturating_add(1);
                    self.truncated = true;
                } else {
                    break;
                }
            }
        }

        fn read(&self, offset: u64, max_len: u64) -> Result<RingChunk, FileRingError> {
            let end = self.end_offset();
            if offset < self.start_offset {
                return Err(FileRingError::OffsetExpired);
            }
            if offset > end {
                return Err(FileRingError::OffsetInFuture);
            }
            let available = end - offset;
            let take = available.min(max_len);
            let begin = (offset - self.start_offset) as usize;
            let data: Vec<u8> = self
                .data
                .iter()
                .skip(begin)
                .take(take as usize)
                .copied()
                .collect();
            let next_offset = offset.saturating_add(take);
            Ok(RingChunk {
                data,
                start_offset: self.start_offset,
                end_offset: end,
                next_offset,
                dropped_bytes: self.dropped_bytes,
                truncated: self.truncated,
                eof: self.eof && next_offset >= end,
            })
        }
    }

    fn scratch_dir() -> PathBuf {
        let base = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let unique = format!(
            "filering-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let dir = base.join(unique);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn paths(dir: &Path) -> (PathBuf, PathBuf) {
        (dir.join("stdout"), dir.join("stdout.meta"))
    }

    #[test]
    fn round_trips_without_drop() {
        let dir = scratch_dir();
        let (data, side) = paths(&dir);
        let mut ring = FileRing::create(&data, &side, 64).unwrap();
        ring.append(b"hello world").unwrap();
        let reader = FileRingReader::open(&data, &side).unwrap();
        let chunk = reader.read(0, 1024).unwrap();
        assert_eq!(chunk.data, b"hello world");
        assert_eq!(chunk.start_offset, 0);
        assert_eq!(chunk.end_offset, 11);
        assert_eq!(chunk.next_offset, 11);
        assert!(!chunk.truncated);
        assert!(!chunk.eof);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn eof_only_observable_when_drained() {
        let dir = scratch_dir();
        let (data, side) = paths(&dir);
        let mut ring = FileRing::create(&data, &side, 64).unwrap();
        ring.append(b"abc").unwrap();
        ring.mark_eof().unwrap();
        let reader = FileRingReader::open(&data, &side).unwrap();
        let partial = reader.read(0, 2).unwrap();
        assert!(!partial.eof, "eof must not show until fully drained");
        let full = reader.read(0, 64).unwrap();
        assert!(full.eof);
        assert_eq!(full.data, b"abc");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn offset_bounds_match_model() {
        let dir = scratch_dir();
        let (data, side) = paths(&dir);
        let mut ring = FileRing::create(&data, &side, 8).unwrap();
        ring.append(b"0123456789").unwrap(); // 10 bytes into cap 8 -> drop 2
        let reader = FileRingReader::open(&data, &side).unwrap();
        assert_eq!(reader.read(0, 4), Err(FileRingError::OffsetExpired));
        assert_eq!(reader.read(11, 4), Err(FileRingError::OffsetInFuture));
        let chunk = reader.read(2, 100).unwrap();
        assert_eq!(chunk.data, b"23456789");
        assert_eq!(chunk.start_offset, 2);
        assert_eq!(chunk.dropped_bytes, 2);
        assert!(chunk.truncated);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lost_flag_ors_into_truncated() {
        let dir = scratch_dir();
        let (data, side) = paths(&dir);
        let mut ring = FileRing::create(&data, &side, 64).unwrap();
        ring.append(b"abc").unwrap();
        ring.mark_lost().unwrap();
        let reader = FileRingReader::open(&data, &side).unwrap();
        let chunk = reader.read(0, 64).unwrap();
        assert!(chunk.truncated, "lost must OR into truncated");
        assert!(chunk.eof);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parity_with_model_under_random_appends() {
        let dir = scratch_dir();
        let (data, side) = paths(&dir);
        let cap = 17u64;
        let mut ring = FileRing::create(&data, &side, cap).unwrap();
        let mut model = ModelRing::new(cap as usize);
        let reader = FileRingReader::open(&data, &side).unwrap();

        // Deterministic pseudo-random byte stream + append sizes.
        let mut state = 0x12345678u32;
        let mut next = || {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            (state >> 16) as u8
        };

        for round in 0..200u32 {
            let len = (next() % 13) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| next()).collect();
            ring.append(&bytes).unwrap();
            model.append(&bytes);

            // Probe several offsets/lengths and compare chunks.
            for probe in 0..4u32 {
                let end = model.end_offset();
                let off = if end == 0 {
                    0
                } else {
                    (next() as u64 + probe as u64) % (end + 1)
                };
                let max_len = (next() as u64 % (cap + 4)).max(1);
                let got = reader.read(off, max_len);
                let want = model.read(off, max_len);
                assert_eq!(
                    got, want,
                    "parity mismatch at round {round} off {off} max {max_len}"
                );
            }
        }

        ring.mark_eof().unwrap();
        model.eof = true;
        let end = model.end_offset();
        assert_eq!(reader.read(end, 8).unwrap(), model.read(end, 8).unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn meta_round_trips() {
        let meta = StreamMeta {
            cap: 4096,
            start_offset: 10,
            end_offset: 4106,
            dropped_bytes: 10,
            truncated: true,
            eof: false,
            lost: false,
        };
        let decoded = StreamMeta::decode(&meta.encode()).unwrap();
        assert_eq!(meta, decoded);
    }

    #[test]
    fn reader_rejects_sidecar_end_ahead_of_data() {
        let dir = scratch_dir();
        let (data, side) = paths(&dir);
        let mut ring = FileRing::create(&data, &side, 64).unwrap();
        ring.append(b"abc").unwrap();
        // Tamper: claim more bytes than the data file actually holds.
        let corrupt = StreamMeta {
            cap: 64,
            start_offset: 0,
            end_offset: 1000,
            dropped_bytes: 0,
            truncated: false,
            eof: false,
            lost: false,
        };
        persist_meta(&side, &corrupt).unwrap();
        let reader = FileRingReader::open(&data, &side).unwrap();
        assert_eq!(reader.meta(), Err(FileRingError::Corrupt));
        assert_eq!(reader.read(0, 64), Err(FileRingError::Corrupt));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reader_rejects_inconsistent_dropped_and_impossible_cap() {
        let dir = scratch_dir();
        let (data, side) = paths(&dir);
        let mut ring = FileRing::create(&data, &side, 64).unwrap();
        ring.append(b"hello").unwrap();
        let reader = FileRingReader::open(&data, &side).unwrap();

        // dropped_bytes drifts from start_offset.
        persist_meta(
            &side,
            &StreamMeta {
                cap: 64,
                start_offset: 0,
                end_offset: 5,
                dropped_bytes: 3,
                truncated: false,
                eof: false,
                lost: false,
            },
        )
        .unwrap();
        assert_eq!(reader.read(0, 64), Err(FileRingError::Corrupt));

        // start > end.
        persist_meta(
            &side,
            &StreamMeta {
                cap: 64,
                start_offset: 9,
                end_offset: 5,
                dropped_bytes: 9,
                truncated: true,
                eof: false,
                lost: false,
            },
        )
        .unwrap();
        assert_eq!(reader.read(0, 64), Err(FileRingError::Corrupt));

        // Impossible cap (above the per-stream retained ceiling).
        persist_meta(
            &side,
            &StreamMeta {
                cap: crate::DETACHED_STREAM_LOG_BYTES + 1,
                start_offset: 0,
                end_offset: 5,
                dropped_bytes: 0,
                truncated: false,
                eof: false,
                lost: false,
            },
        )
        .unwrap();
        assert_eq!(reader.read(0, 64), Err(FileRingError::Corrupt));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ring_chunk_debug_redacts_payload_bytes() {
        let chunk = RingChunk {
            data: b"super secret stdout".to_vec(),
            start_offset: 3,
            end_offset: 22,
            next_offset: 22,
            dropped_bytes: 3,
            truncated: true,
            eof: true,
        };
        let rendered = format!("{chunk:?}");
        assert!(!rendered.contains("secret"), "payload must never appear: {rendered}");
        assert!(rendered.contains("data_len"));
        assert!(rendered.contains("19"));
    }
}
