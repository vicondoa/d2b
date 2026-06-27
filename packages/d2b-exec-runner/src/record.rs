//! Durable record + runner status phase markers (binary, no serde).
//!
//! - The `record` file is written and read by guestd (the detached registry's
//!   canonical crash-recovery state). The runner never touches it.
//! - The `status` file is written by the runner (the supervisor) and read by
//!   guestd to learn the exec's lifecycle phase.

use crate::codec::{DecodeError, Reader, Writer};

const RECORD_MAGIC: u32 = 0x4e4c_4552; // "NLER"
const RECORD_VERSION: u32 = 1;
/// Marks a record as a detached exec record so a lookup can never cross
/// authorization domains (an attached id can never resolve one of these).
const RECORD_KIND_DETACHED: u8 = 1;

const STATUS_MAGIC: u32 = 0x4e4c_5354; // "NLST"
const STATUS_VERSION: u32 = 1;

/// Linear durable record state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordState {
    /// Record + spec written and fsync'd BEFORE `systemd-run`; the durable
    /// "intent to start a unit for this slot".
    Dispatching,
    /// Runner published a `started` status.
    Running,
    /// Child exited with a code.
    Exited,
    /// Child was terminated by a signal.
    Signaled,
    /// Cancelled (explicit cancel, ceiling, or vanished-unit `lost`).
    Cancelled,
    /// The runner could not spawn the child (legitimate terminal exec).
    SpawnFailed,
}

impl RecordState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            RecordState::Exited
                | RecordState::Signaled
                | RecordState::Cancelled
                | RecordState::SpawnFailed
        )
    }

    fn tag(self) -> u8 {
        match self {
            RecordState::Dispatching => 0,
            RecordState::Running => 1,
            RecordState::Exited => 2,
            RecordState::Signaled => 3,
            RecordState::Cancelled => 4,
            RecordState::SpawnFailed => 5,
        }
    }

    fn from_tag(tag: u8) -> Result<Self, DecodeError> {
        Ok(match tag {
            0 => RecordState::Dispatching,
            1 => RecordState::Running,
            2 => RecordState::Exited,
            3 => RecordState::Signaled,
            4 => RecordState::Cancelled,
            5 => RecordState::SpawnFailed,
            _ => return Err(DecodeError::InvalidTag),
        })
    }
}

/// The canonical guestd-owned detached record.
#[derive(Clone, PartialEq, Eq)]
pub struct DurableRecord {
    pub exec_id: String,
    pub slot: u32,
    pub boot_id: String,
    pub create_time_unix: u64,
    pub dispatch_deadline_unix: u64,
    pub argv_sha256: String,
    pub state: RecordState,
    pub exit_code: Option<i32>,
    pub term_signal: Option<u32>,
    /// True when the record became `Cancelled` because its unit/runner
    /// vanished without a terminal status (live reconciliation).
    pub lost: bool,
    /// Wall-clock time the record first reached a terminal state (for TTL).
    pub terminal_time_unix: Option<u64>,
}

// Redacted Debug: never echo the exec id or argv hash provenance beyond shape.
impl std::fmt::Debug for DurableRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DurableRecord")
            .field("slot", &self.slot)
            .field("state", &self.state)
            .field("lost", &self.lost)
            .field("terminal_time_unix", &self.terminal_time_unix)
            .finish()
    }
}

impl DurableRecord {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.put_u32(RECORD_MAGIC);
        w.put_u32(RECORD_VERSION);
        w.put_u8(RECORD_KIND_DETACHED);
        w.put_str(&self.exec_id);
        w.put_u32(self.slot);
        w.put_str(&self.boot_id);
        w.put_u64(self.create_time_unix);
        w.put_u64(self.dispatch_deadline_unix);
        w.put_str(&self.argv_sha256);
        w.put_u8(self.state.tag());
        put_opt_i32(&mut w, self.exit_code);
        put_opt_u32(&mut w, self.term_signal);
        w.put_bool(self.lost);
        put_opt_u64(&mut w, self.terminal_time_unix);
        w.into_vec()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(bytes);
        if r.get_u32()? != RECORD_MAGIC {
            return Err(DecodeError::BadMagic);
        }
        if r.get_u32()? != RECORD_VERSION {
            return Err(DecodeError::BadVersion);
        }
        if r.get_u8()? != RECORD_KIND_DETACHED {
            return Err(DecodeError::InvalidTag);
        }
        let exec_id = r.get_str()?;
        let slot = r.get_u32()?;
        let boot_id = r.get_str()?;
        let create_time_unix = r.get_u64()?;
        let dispatch_deadline_unix = r.get_u64()?;
        let argv_sha256 = r.get_str()?;
        let state = RecordState::from_tag(r.get_u8()?)?;
        let exit_code = get_opt_i32(&mut r)?;
        let term_signal = get_opt_u32(&mut r)?;
        let lost = r.get_bool()?;
        let terminal_time_unix = get_opt_u64(&mut r)?;
        r.finish()?;
        Ok(Self {
            exec_id,
            slot,
            boot_id,
            create_time_unix,
            dispatch_deadline_unix,
            argv_sha256,
            state,
            exit_code,
            term_signal,
            lost,
            terminal_time_unix,
        })
    }
}

/// Runner-written lifecycle phase marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusPhase {
    /// Child spawned successfully; supervisor is resident.
    Started,
    /// Spawn failed (ENOENT/EACCES/exec-format/...). Terminal, retained.
    SpawnFailed,
    /// Spec-parse / dir / log-open infra failure. The runner did NOT spawn;
    /// guestd treats this as a create error and cleans up.
    InfraFailed,
    /// Child exited with a status code.
    Exited(i32),
    /// Child terminated by a signal.
    Signaled(i32),
    /// Cancel/stop path completed (TERM->grace->KILL->reap).
    Cancelled,
}

impl StatusPhase {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            StatusPhase::SpawnFailed
                | StatusPhase::InfraFailed
                | StatusPhase::Exited(_)
                | StatusPhase::Signaled(_)
                | StatusPhase::Cancelled
        )
    }

    fn tag(self) -> u8 {
        match self {
            StatusPhase::Started => 0,
            StatusPhase::SpawnFailed => 1,
            StatusPhase::InfraFailed => 2,
            StatusPhase::Exited(_) => 3,
            StatusPhase::Signaled(_) => 4,
            StatusPhase::Cancelled => 5,
        }
    }
}

/// Wrapper carrying the phase (kept distinct from [`StatusPhase`] so future
/// status metadata can be added without churning call sites).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusRecord {
    pub phase: StatusPhase,
}

impl StatusRecord {
    pub fn new(phase: StatusPhase) -> Self {
        Self { phase }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.put_u32(STATUS_MAGIC);
        w.put_u32(STATUS_VERSION);
        w.put_u8(self.phase.tag());
        match self.phase {
            StatusPhase::Exited(code) => w.put_i32(code),
            StatusPhase::Signaled(signal) => w.put_i32(signal),
            _ => {}
        }
        w.into_vec()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(bytes);
        if r.get_u32()? != STATUS_MAGIC {
            return Err(DecodeError::BadMagic);
        }
        if r.get_u32()? != STATUS_VERSION {
            return Err(DecodeError::BadVersion);
        }
        let tag = r.get_u8()?;
        let phase = match tag {
            0 => StatusPhase::Started,
            1 => StatusPhase::SpawnFailed,
            2 => StatusPhase::InfraFailed,
            3 => StatusPhase::Exited(r.get_i32()?),
            4 => StatusPhase::Signaled(r.get_i32()?),
            5 => StatusPhase::Cancelled,
            _ => return Err(DecodeError::InvalidTag),
        };
        r.finish()?;
        Ok(Self { phase })
    }
}

fn put_opt_i32(w: &mut Writer, value: Option<i32>) {
    match value {
        Some(v) => {
            w.put_bool(true);
            w.put_i32(v);
        }
        None => w.put_bool(false),
    }
}

fn put_opt_u32(w: &mut Writer, value: Option<u32>) {
    match value {
        Some(v) => {
            w.put_bool(true);
            w.put_u32(v);
        }
        None => w.put_bool(false),
    }
}

fn put_opt_u64(w: &mut Writer, value: Option<u64>) {
    match value {
        Some(v) => {
            w.put_bool(true);
            w.put_u64(v);
        }
        None => w.put_bool(false),
    }
}

fn get_opt_i32(r: &mut Reader<'_>) -> Result<Option<i32>, DecodeError> {
    if r.get_bool()? {
        Ok(Some(r.get_i32()?))
    } else {
        Ok(None)
    }
}

fn get_opt_u32(r: &mut Reader<'_>) -> Result<Option<u32>, DecodeError> {
    if r.get_bool()? {
        Ok(Some(r.get_u32()?))
    } else {
        Ok(None)
    }
}

fn get_opt_u64(r: &mut Reader<'_>) -> Result<Option<u64>, DecodeError> {
    if r.get_bool()? {
        Ok(Some(r.get_u64()?))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record() -> DurableRecord {
        DurableRecord {
            exec_id: "00112233445566778899aabbccddeeff".to_owned(),
            slot: 3,
            boot_id: "boot-xyz".to_owned(),
            create_time_unix: 1_700_000_000,
            dispatch_deadline_unix: 1_700_000_030,
            argv_sha256: "f".repeat(64),
            state: RecordState::Running,
            exit_code: None,
            term_signal: None,
            lost: false,
            terminal_time_unix: None,
        }
    }

    #[test]
    fn record_round_trips() {
        let record = sample_record();
        let bytes = record.encode();
        let decoded = DurableRecord::decode(&bytes).unwrap();
        assert!(record == decoded);
    }

    #[test]
    fn record_round_trips_terminal() {
        let mut record = sample_record();
        record.state = RecordState::Exited;
        record.exit_code = Some(42);
        record.terminal_time_unix = Some(1_700_000_100);
        let decoded = DurableRecord::decode(&record.encode()).unwrap();
        assert!(record == decoded);
        assert!(decoded.state.is_terminal());

        let mut record = sample_record();
        record.state = RecordState::Cancelled;
        record.lost = true;
        record.terminal_time_unix = Some(1_700_000_200);
        let decoded = DurableRecord::decode(&record.encode()).unwrap();
        assert!(record == decoded);
    }

    #[test]
    fn record_rejects_bad_magic() {
        let mut bytes = sample_record().encode();
        bytes[0] ^= 0xff;
        assert_eq!(DurableRecord::decode(&bytes), Err(DecodeError::BadMagic));
    }

    #[test]
    fn status_round_trips_each_phase() {
        for phase in [
            StatusPhase::Started,
            StatusPhase::SpawnFailed,
            StatusPhase::InfraFailed,
            StatusPhase::Exited(7),
            StatusPhase::Signaled(9),
            StatusPhase::Cancelled,
        ] {
            let record = StatusRecord::new(phase);
            let decoded = StatusRecord::decode(&record.encode()).unwrap();
            assert_eq!(record, decoded);
        }
        assert!(!StatusPhase::Started.is_terminal());
        assert!(StatusPhase::Exited(0).is_terminal());
    }

    #[test]
    fn record_debug_is_redacted() {
        let record = sample_record();
        let rendered = format!("{record:?}");
        assert!(!rendered.contains("00112233"));
        assert!(!rendered.contains("boot-xyz"));
    }
}
