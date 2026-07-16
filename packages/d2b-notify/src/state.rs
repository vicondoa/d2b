// SPDX-License-Identifier: Apache-2.0
//! Bounded presentation projection for desktop status consumers.
//!
//! A ComponentSession observer may materialize this read model for Waybar or
//! other desktop renderers. The file is never an endpoint, authorization
//! source, repair input, or fallback control channel.
//!
//! ## File format
//!
//! The file is a single JSON object versioned by `schemaVersion`.  Readers
//! MUST ignore unknown top-level fields to preserve forward compatibility.
//! Readers MUST reject a `schemaVersion` they do not understand.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::events::SecurityKeyEvent;

/// Current schema version for `sk-state.json`.
pub const STATE_SCHEMA_VERSION: u32 = 1;
pub const MAX_PROJECTION_BYTES: usize = 32 * 1024;
pub const MAX_ACTIVE_CEREMONIES: usize = 16;
pub const MAX_PROJECTION_SESSION_ID_CHARS: usize = 64;
pub const MAX_PROJECTION_VM_NAME_CHARS: usize = 64;
pub const MAX_PROJECTION_RP_ID_CHARS: usize = 128;
pub const MAX_EVENT_KIND_CHARS: usize = 32;

/// Summary of one security-key ceremony, as recorded in the durable state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CeremonySummary {
    /// Opaque ceremony session identifier (hex token).
    pub session_id: String,
    /// VM name that requested the ceremony.
    pub vm_name: String,
    /// Relying-party identifier if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rp_id: Option<String>,
    /// Last event kind recorded for this session (e.g. `"started"`).
    pub last_event_kind: String,
    /// Unix timestamp (seconds) when the last event was recorded.
    pub last_event_at: u64,
    /// Whether the session is in a terminal state.
    pub is_terminal: bool,
}

impl CeremonySummary {
    /// Build a summary from the latest event.
    pub fn from_event(event: &SecurityKeyEvent, now_secs: u64) -> Self {
        let kind = serde_json::to_value(event)
            .ok()
            .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_owned))
            .unwrap_or_else(|| "unknown".to_owned());
        let rp_id = match event {
            SecurityKeyEvent::Started { rp_id, .. } => rp_id.clone(),
            _ => None,
        };
        Self {
            session_id: projection_text(event.session_id(), MAX_PROJECTION_SESSION_ID_CHARS),
            vm_name: projection_text(event.vm_name(), MAX_PROJECTION_VM_NAME_CHARS),
            rp_id: rp_id.map(|value| projection_text(&value, MAX_PROJECTION_RP_ID_CHARS)),
            last_event_kind: projection_text(&kind, MAX_EVENT_KIND_CHARS),
            last_event_at: now_secs,
            is_terminal: event.is_terminal(),
        }
    }
}

/// Durable state file contents written by the host runtime and read by
/// the Waybar helper and wlcontrol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SkNotifyState {
    /// Must equal [`STATE_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Unix timestamp (seconds) when this state was last written.
    pub updated_at: u64,
    /// All active (non-terminal) ceremonies.
    pub active: Vec<CeremonySummary>,
    /// Recent terminal ceremonies (bounded ring, most recent first).
    pub recent_terminal: Vec<CeremonySummary>,
}

impl SkNotifyState {
    /// Maximum number of terminal entries retained in `recent_terminal`.
    pub const MAX_RECENT_TERMINAL: usize = 8;
    /// Active entries older than this are ignored by status consumers unless a
    /// newer broker event refreshes them.
    pub const ACTIVE_STALE_AFTER_SECS: u64 = 300;

    /// Construct an empty state.
    pub fn empty(now_secs: u64) -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            updated_at: now_secs,
            active: vec![],
            recent_terminal: vec![],
        }
    }

    /// Apply a new event to the state and return the updated state.
    ///
    /// The caller is responsible for writing the updated state to disk.
    pub fn apply(mut self, event: &SecurityKeyEvent, now_secs: u64) -> Self {
        self.prune_stale_active(now_secs);
        self.updated_at = now_secs;
        let summary = CeremonySummary::from_event(event, now_secs);

        if event.is_terminal() {
            // Remove from active (if present) and prepend to recent_terminal.
            self.active.retain(|s| s.session_id != event.session_id());
            self.recent_terminal.insert(0, summary);
            self.recent_terminal.truncate(Self::MAX_RECENT_TERMINAL);
        } else {
            // Update-or-insert the active entry.
            if let Some(pos) = self
                .active
                .iter()
                .position(|s| s.session_id == event.session_id())
            {
                self.active[pos] = summary;
            } else {
                self.active.push(summary);
            }
            self.active.sort_by_key(|entry| entry.last_event_at);
            if self.active.len() > MAX_ACTIVE_CEREMONIES {
                let excess = self.active.len() - MAX_ACTIVE_CEREMONIES;
                self.active.drain(..excess);
            }
        }
        self
    }

    /// Drop active entries whose last event is too old to represent a live
    /// ceremony. This keeps Waybar/wlcontrol from showing a stuck active key
    /// forever if the broker crashes before emitting a terminal event.
    pub fn prune_stale_active(&mut self, now_secs: u64) {
        self.active
            .retain(|s| now_secs.saturating_sub(s.last_event_at) <= Self::ACTIVE_STALE_AFTER_SECS);
    }

    /// Serialize to a compact JSON string.
    pub fn to_json(&self) -> Result<String, StateWriteError> {
        self.validate()
            .map_err(StateWriteError::InvalidProjection)?;
        let encoded = serde_json::to_string(self).map_err(StateWriteError::Json)?;
        if encoded.len() > MAX_PROJECTION_BYTES {
            return Err(StateWriteError::ProjectionTooLarge);
        }
        Ok(encoded)
    }

    /// Deserialize from a JSON string.  Returns an error if the schema
    /// version is not [`STATE_SCHEMA_VERSION`].
    pub fn from_json(s: &str) -> Result<Self, StateReadError> {
        if s.len() > MAX_PROJECTION_BYTES {
            return Err(StateReadError::ProjectionTooLarge);
        }
        let state: Self = serde_json::from_str(s).map_err(StateReadError::Json)?;
        if state.schema_version != STATE_SCHEMA_VERSION {
            return Err(StateReadError::UnsupportedVersion(state.schema_version));
        }
        state
            .validate()
            .map_err(StateReadError::InvalidProjection)?;
        Ok(state)
    }

    pub fn from_slice(bytes: &[u8]) -> Result<Self, StateReadError> {
        if bytes.len() > MAX_PROJECTION_BYTES {
            return Err(StateReadError::ProjectionTooLarge);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| StateReadError::InvalidEncoding)?;
        Self::from_json(text)
    }

    /// True if there are any active (non-terminal) ceremonies.
    pub fn has_active(&self) -> bool {
        !self.active.is_empty()
    }

    fn validate(&self) -> Result<(), ProjectionValidationError> {
        if self.active.len() > MAX_ACTIVE_CEREMONIES
            || self.recent_terminal.len() > Self::MAX_RECENT_TERMINAL
        {
            return Err(ProjectionValidationError::EntryLimit);
        }
        for summary in self.active.iter().chain(&self.recent_terminal) {
            validate_text(&summary.session_id, MAX_PROJECTION_SESSION_ID_CHARS, false)?;
            validate_text(&summary.vm_name, MAX_PROJECTION_VM_NAME_CHARS, false)?;
            if let Some(rp_id) = &summary.rp_id {
                validate_text(rp_id, MAX_PROJECTION_RP_ID_CHARS, true)?;
            }
            validate_text(&summary.last_event_kind, MAX_EVENT_KIND_CHARS, false)?;
        }
        Ok(())
    }
}

/// Error reading the durable state file.
#[derive(Debug)]
pub enum StateReadError {
    Json(serde_json::Error),
    UnsupportedVersion(u32),
    ProjectionTooLarge,
    InvalidEncoding,
    InvalidProjection(ProjectionValidationError),
}

impl std::fmt::Display for StateReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "invalid JSON: {e}"),
            Self::UnsupportedVersion(v) => {
                write!(f, "unsupported sk-state.json schema version {v}")
            }
            Self::ProjectionTooLarge => f.write_str("state projection exceeds size limit"),
            Self::InvalidEncoding => f.write_str("state projection is not UTF-8"),
            Self::InvalidProjection(error) => write!(f, "invalid state projection: {error}"),
        }
    }
}

impl std::error::Error for StateReadError {}

#[derive(Debug)]
pub enum StateWriteError {
    Json(serde_json::Error),
    ProjectionTooLarge,
    InvalidProjection(ProjectionValidationError),
}

impl std::fmt::Display for StateWriteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(_) => formatter.write_str("cannot encode state projection"),
            Self::ProjectionTooLarge => formatter.write_str("state projection exceeds size limit"),
            Self::InvalidProjection(error) => {
                write!(formatter, "invalid state projection: {error}")
            }
        }
    }
}

impl std::error::Error for StateWriteError {}

#[derive(Debug)]
pub enum ProjectionValidationError {
    EntryLimit,
    InvalidText,
}

impl std::fmt::Display for ProjectionValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EntryLimit => formatter.write_str("entry count exceeds limit"),
            Self::InvalidText => formatter.write_str("text field exceeds limit"),
        }
    }
}

fn projection_text(input: &str, max_chars: usize) -> String {
    input
        .chars()
        .take(max_chars)
        .map(|character| {
            if character.is_control() {
                '\u{fffd}'
            } else {
                character
            }
        })
        .collect()
}

fn validate_text(
    input: &str,
    max_chars: usize,
    allow_empty: bool,
) -> Result<(), ProjectionValidationError> {
    if (!allow_empty && input.is_empty())
        || input.chars().count() > max_chars
        || input.chars().any(char::is_control)
    {
        Err(ProjectionValidationError::InvalidText)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{BusyDetail, SecurityKeyEvent};

    const T0: u64 = 1_750_000_000;

    fn started(session_id: &str, vm: &str) -> SecurityKeyEvent {
        SecurityKeyEvent::Started {
            session_id: session_id.to_owned(),
            vm_name: vm.to_owned(),
            rp_id: None,
        }
    }

    fn completed(session_id: &str, vm: &str) -> SecurityKeyEvent {
        SecurityKeyEvent::Completed {
            session_id: session_id.to_owned(),
            vm_name: vm.to_owned(),
        }
    }

    fn timed_out(session_id: &str, vm: &str) -> SecurityKeyEvent {
        SecurityKeyEvent::TimedOut {
            session_id: session_id.to_owned(),
            vm_name: vm.to_owned(),
        }
    }

    #[test]
    fn empty_state_round_trips() {
        let state = SkNotifyState::empty(T0);
        let json = state.to_json().unwrap();
        let decoded = SkNotifyState::from_json(&json).unwrap();
        assert_eq!(decoded, state);
    }

    #[test]
    fn started_event_adds_active_entry() {
        let state = SkNotifyState::empty(T0).apply(&started("s1", "personal-dev"), T0 + 1);
        assert_eq!(state.active.len(), 1);
        assert_eq!(state.active[0].session_id, "s1");
        assert_eq!(state.active[0].vm_name, "personal-dev");
        assert!(!state.active[0].is_terminal);
        assert!(state.recent_terminal.is_empty());
    }

    #[test]
    fn touch_needed_updates_existing_active_entry() {
        let state = SkNotifyState::empty(T0)
            .apply(&started("s1", "vm1"), T0)
            .apply(
                &SecurityKeyEvent::TouchNeeded {
                    session_id: "s1".to_owned(),
                    vm_name: "vm1".to_owned(),
                },
                T0 + 5,
            );
        assert_eq!(state.active.len(), 1, "must not duplicate active entry");
        assert_eq!(state.active[0].last_event_kind, "touchNeeded");
    }

    #[test]
    fn terminal_event_moves_entry_from_active_to_recent() {
        let state = SkNotifyState::empty(T0)
            .apply(&started("s1", "vm1"), T0)
            .apply(&completed("s1", "vm1"), T0 + 10);
        assert!(state.active.is_empty());
        assert_eq!(state.recent_terminal.len(), 1);
        assert_eq!(state.recent_terminal[0].session_id, "s1");
        assert!(state.recent_terminal[0].is_terminal);
    }

    #[test]
    fn recent_terminal_ring_is_bounded() {
        let mut state = SkNotifyState::empty(T0);
        for i in 0..(SkNotifyState::MAX_RECENT_TERMINAL + 3) {
            let sid = format!("s{i}");
            state = state
                .apply(&started(&sid, "vm"), T0 + i as u64)
                .apply(&completed(&sid, "vm"), T0 + i as u64 + 1);
        }
        assert!(
            state.recent_terminal.len() <= SkNotifyState::MAX_RECENT_TERMINAL,
            "recent_terminal must be bounded to {}",
            SkNotifyState::MAX_RECENT_TERMINAL
        );
    }

    #[test]
    fn active_projection_is_bounded_and_keeps_newest_entries() {
        let mut state = SkNotifyState::empty(T0);
        for index in 0..(MAX_ACTIVE_CEREMONIES + 3) {
            state = state.apply(&started(&format!("s{index}"), "vm"), T0 + index as u64);
        }
        assert_eq!(state.active.len(), MAX_ACTIVE_CEREMONIES);
        assert_eq!(state.active[0].session_id, "s3");
    }

    #[test]
    fn recent_terminal_is_most_recent_first() {
        let state = SkNotifyState::empty(T0)
            .apply(&started("s1", "vm"), T0)
            .apply(&completed("s1", "vm"), T0 + 1)
            .apply(&started("s2", "vm"), T0 + 2)
            .apply(&timed_out("s2", "vm"), T0 + 3);
        assert_eq!(state.recent_terminal[0].session_id, "s2");
        assert_eq!(state.recent_terminal[1].session_id, "s1");
    }

    #[test]
    fn stale_active_entries_are_pruned_when_applying_events() {
        let state = SkNotifyState::empty(T0)
            .apply(&started("stale", "vm1"), T0)
            .apply(
                &started("fresh", "vm2"),
                T0 + SkNotifyState::ACTIVE_STALE_AFTER_SECS + 1,
            );

        assert_eq!(state.active.len(), 1);
        assert_eq!(state.active[0].session_id, "fresh");
    }

    #[test]
    fn explicit_prune_removes_stale_active_entries() {
        let mut state = SkNotifyState::empty(T0).apply(&started("stale", "vm1"), T0);

        state.prune_stale_active(T0 + SkNotifyState::ACTIVE_STALE_AFTER_SECS + 1);

        assert!(state.active.is_empty());
    }

    #[test]
    fn unsupported_schema_version_is_rejected() {
        let json = r#"{"schemaVersion":99,"updatedAt":0,"active":[],"recentTerminal":[]}"#;
        let err = SkNotifyState::from_json(json).unwrap_err();
        assert!(
            matches!(err, StateReadError::UnsupportedVersion(99)),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn malformed_json_is_rejected() {
        let err = SkNotifyState::from_json("{not json}").unwrap_err();
        assert!(matches!(err, StateReadError::Json(_)));
    }

    #[test]
    fn oversized_projection_is_rejected_before_decode() {
        let input = vec![b' '; MAX_PROJECTION_BYTES + 1];
        assert!(matches!(
            SkNotifyState::from_slice(&input),
            Err(StateReadError::ProjectionTooLarge)
        ));
    }

    #[test]
    fn projection_text_is_sanitized_and_bounded() {
        let state =
            SkNotifyState::empty(T0).apply(&started("s1", &format!("vm\n{}", "x".repeat(100))), T0);
        assert!(state.active[0].vm_name.chars().count() <= MAX_PROJECTION_VM_NAME_CHARS);
        assert!(!state.active[0].vm_name.contains('\n'));
        assert!(state.to_json().unwrap().len() <= MAX_PROJECTION_BYTES);
    }

    #[test]
    fn state_json_round_trips_with_busy_event() {
        let state = SkNotifyState::empty(T0).apply(
            &SecurityKeyEvent::Busy {
                session_id: "s1".to_owned(),
                vm_name: "work-aad".to_owned(),
                detail: BusyDetail {
                    holder_vm: "personal-dev".to_owned(),
                    waiting_vms: vec!["other-vm".to_owned()],
                },
            },
            T0,
        );
        let json = state.to_json().unwrap();
        let decoded = SkNotifyState::from_json(&json).unwrap();
        assert_eq!(decoded.active[0].last_event_kind, "busy");
    }

    #[test]
    fn ceremony_summary_from_started_event_captures_rp_id() {
        let event = SecurityKeyEvent::Started {
            session_id: "s".to_owned(),
            vm_name: "vm".to_owned(),
            rp_id: Some("github.com".to_owned()),
        };
        let summary = CeremonySummary::from_event(&event, T0);
        assert_eq!(summary.rp_id, Some("github.com".to_owned()));
        assert_eq!(summary.last_event_kind, "started");
    }
}
