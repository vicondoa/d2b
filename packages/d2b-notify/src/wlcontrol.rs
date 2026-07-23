// SPDX-License-Identifier: Apache-2.0
//! Data contract for the `d2b-wlcontrol` status/action surface.
//!
//! `d2b-wlcontrol` reads the durable security-key state and renders it as a
//! status row or panel.  This module defines the typed DTO that wlcontrol
//! reads — it is produced by the host runtime and consumed by wlcontrol.
//!
//! ## Rendering contract
//!
//! wlcontrol MUST read `SkNotifyState` from the state file and call
//! [`WlcontrolSkStatus::from_state`] to obtain the surface DTO.  It MUST NOT
//! embed business logic about ceremony lifecycle; all lifecycle logic lives in
//! `d2b-notify`.
//!
//! ## Action routing
//!
//! Each [`WlcontrolAction`] carries a pre-minted nonce token in `action_key`
//! (see [`crate::nonce`]).  When the user triggers an action, wlcontrol MUST
//! forward it as a `d2b usb security-key cancel --action-token <token>` CLI
//! invocation (or the equivalent daemon RPC) and MUST NOT perform any
//! privileged host mutation directly.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::state::{CeremonySummary, SkNotifyState};

/// Overall status of the USB security-key subsystem as presented in wlcontrol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SkOverallStatus {
    /// No active ceremonies; subsystem healthy.
    Idle,
    /// One or more active ceremonies in progress.
    Active,
    /// At least one ceremony requires physical touch.
    TouchNeeded,
    /// At least one ceremony is waiting because the key is busy.
    Busy,
    /// At least one ceremony was blocked or failed.
    Error,
}

/// A single actionable item in the wlcontrol action list.
///
/// The `action_key` is the full `d2b-sk-<verb>:<nonce>` string; wlcontrol
/// must pass it verbatim to the CLI or daemon without modifying it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WlcontrolAction {
    /// Full action key including the nonce, as produced by
    /// [`crate::nonce::action_key_for`].
    pub action_key: String,
    /// Human-readable label for the action button.
    pub label: String,
    /// Session this action applies to.
    pub session_id: String,
}

/// Per-ceremony row in the wlcontrol panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WlcontrolCeremonyRow {
    pub session_id: String,
    pub vm_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rp_id: Option<String>,
    /// Human-readable status label.
    pub status_label: String,
    /// Actions available for this ceremony.
    pub actions: Vec<WlcontrolAction>,
}

/// Root DTO for the wlcontrol security-key status/action surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WlcontrolSkStatus {
    /// Overall status used to drive panel title / icon.
    pub overall: SkOverallStatus,
    /// Active ceremonies with per-row actions.
    pub active: Vec<WlcontrolCeremonyRow>,
    /// Recent terminal ceremonies (informational, no actions).
    pub recent_terminal: Vec<CeremonySummary>,
}

impl WlcontrolSkStatus {
    /// Derive the wlcontrol DTO from the durable state.
    ///
    /// `action_builder` is a callback invoked per active ceremony; it returns
    /// zero or more [`WlcontrolAction`]s for that ceremony (e.g. a "Cancel"
    /// button backed by a freshly minted nonce).  Pass `|_| vec![]` when no
    /// actions are available or when tests want a simpler fixture.
    pub fn from_state<F>(state: &SkNotifyState, mut action_builder: F) -> Self
    where
        F: FnMut(&CeremonySummary) -> Vec<WlcontrolAction>,
    {
        let overall = derive_overall_status(state);
        let active = state
            .active
            .iter()
            .map(|summary| {
                let status_label = summary_status_label(summary);
                let actions = action_builder(summary);
                WlcontrolCeremonyRow {
                    session_id: summary.session_id.clone(),
                    vm_name: summary.vm_name.clone(),
                    rp_id: summary.rp_id.clone(),
                    status_label,
                    actions,
                }
            })
            .collect();
        WlcontrolSkStatus {
            overall,
            active,
            recent_terminal: state.recent_terminal.clone(),
        }
    }
}

fn derive_overall_status(state: &SkNotifyState) -> SkOverallStatus {
    if state.active.is_empty() {
        return SkOverallStatus::Idle;
    }
    if state
        .active
        .iter()
        .any(|s| s.last_event_kind == "touchNeeded")
    {
        return SkOverallStatus::TouchNeeded;
    }
    if state.active.iter().any(|s| s.last_event_kind == "busy") {
        return SkOverallStatus::Busy;
    }
    if state
        .active
        .iter()
        .any(|s| matches!(s.last_event_kind.as_str(), "blocked" | "failed"))
    {
        return SkOverallStatus::Error;
    }
    SkOverallStatus::Active
}

fn summary_status_label(summary: &CeremonySummary) -> String {
    match summary.last_event_kind.as_str() {
        "started" => "Authenticating…".to_owned(),
        "touchNeeded" => "Needs touch".to_owned(),
        "busy" => "Waiting (key busy)".to_owned(),
        "queued" => "Queued".to_owned(),
        "blocked" => "Blocked".to_owned(),
        "timedOut" => "Timed out".to_owned(),
        "failed" => "Failed".to_owned(),
        "canceled" => "Canceled".to_owned(),
        "completed" => "Completed".to_owned(),
        other => other.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::SecurityKeyEvent;
    use crate::state::SkNotifyState;

    const T0: u64 = 1_750_000_000;

    #[test]
    fn idle_state_is_idle() {
        let state = SkNotifyState::empty(T0);
        let status = WlcontrolSkStatus::from_state(&state, |_| vec![]);
        assert_eq!(status.overall, SkOverallStatus::Idle);
        assert!(status.active.is_empty());
    }

    #[test]
    fn started_ceremony_is_active() {
        let state = SkNotifyState::empty(T0).apply(
            &SecurityKeyEvent::Started {
                session_id: "s1".to_owned(),
                vm_name: "personal-dev".to_owned(),
                rp_id: Some("github.com".to_owned()),
            },
            T0,
        );
        let status = WlcontrolSkStatus::from_state(&state, |_| vec![]);
        assert_eq!(status.overall, SkOverallStatus::Active);
        assert_eq!(status.active[0].rp_id.as_deref(), Some("github.com"));
        assert_eq!(status.active[0].status_label, "Authenticating…");
    }

    #[test]
    fn touch_needed_becomes_touch_needed_status() {
        let state = SkNotifyState::empty(T0).apply(
            &SecurityKeyEvent::TouchNeeded {
                session_id: "s1".to_owned(),
                vm_name: "vm1".to_owned(),
            },
            T0,
        );
        let status = WlcontrolSkStatus::from_state(&state, |_| vec![]);
        assert_eq!(status.overall, SkOverallStatus::TouchNeeded);
        assert_eq!(status.active[0].status_label, "Needs touch");
    }

    #[test]
    fn action_builder_is_invoked_per_active_ceremony() {
        let state = SkNotifyState::empty(T0)
            .apply(
                &SecurityKeyEvent::Started {
                    session_id: "s1".to_owned(),
                    vm_name: "vm1".to_owned(),
                    rp_id: None,
                },
                T0,
            )
            .apply(
                &SecurityKeyEvent::Started {
                    session_id: "s2".to_owned(),
                    vm_name: "vm2".to_owned(),
                    rp_id: None,
                },
                T0 + 1,
            );
        let mut invocations = 0usize;
        let _status = WlcontrolSkStatus::from_state(&state, |_| {
            invocations += 1;
            vec![]
        });
        assert_eq!(
            invocations, 2,
            "action builder must be called once per active ceremony"
        );
    }

    #[test]
    fn wlcontrol_status_round_trips_via_json() {
        let state = SkNotifyState::empty(T0).apply(
            &SecurityKeyEvent::Started {
                session_id: "s1".to_owned(),
                vm_name: "vm1".to_owned(),
                rp_id: None,
            },
            T0,
        );
        let action = WlcontrolAction {
            action_key: "d2b-sk-cancel:".to_owned() + &"a".repeat(64),
            label: "Cancel request".to_owned(),
            session_id: "s1".to_owned(),
        };
        let status = WlcontrolSkStatus::from_state(&state, |_| vec![action.clone()]);
        let json = serde_json::to_string(&status).unwrap();
        let decoded: WlcontrolSkStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.active[0].actions[0].action_key, action.action_key);
    }
}
