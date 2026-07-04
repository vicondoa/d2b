// SPDX-License-Identifier: Apache-2.0
//! Durable JSON state format for the d2b notification layer.
//!
//! The host runtime writes `sk-state.json` to
//! `/run/d2b/notify/` (the `d2b.notifications.runtime.stateDir` path)
//! whenever the set of active or recently terminal security-key ceremonies
//! changes.  The Waybar helper and `d2b-wlcontrol` read this file on demand.
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
            session_id: event.session_id().to_owned(),
            vm_name: event.vm_name().to_owned(),
            rp_id,
            last_event_kind: kind,
            last_event_at: now_secs,
            is_terminal: event.is_terminal(),
        }
    }
}

/// Durable state file contents written by the host runtime and read by
/// the Waybar helper and wlcontrol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
        }
        self
    }

    /// Serialize to a compact JSON string.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }

    /// Deserialize from a JSON string.  Returns an error if the schema
    /// version is not [`STATE_SCHEMA_VERSION`].
    pub fn from_json(s: &str) -> Result<Self, StateReadError> {
        let state: Self = serde_json::from_str(s).map_err(StateReadError::Json)?;
        if state.schema_version != STATE_SCHEMA_VERSION {
            return Err(StateReadError::UnsupportedVersion(state.schema_version));
        }
        Ok(state)
    }

    /// True if there are any active (non-terminal) ceremonies.
    pub fn has_active(&self) -> bool {
        !self.active.is_empty()
    }
}

/// Error reading the durable state file.
#[derive(Debug)]
pub enum StateReadError {
    Json(serde_json::Error),
    UnsupportedVersion(u32),
}

impl std::fmt::Display for StateReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "invalid JSON: {e}"),
            Self::UnsupportedVersion(v) => {
                write!(f, "unsupported sk-state.json schema version {v}")
            }
        }
    }
}

impl std::error::Error for StateReadError {}

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
