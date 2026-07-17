// SPDX-License-Identifier: Apache-2.0
//! Presentation DTO for an authenticated `d2b-wlcontrol` action client.
//!
//! The observer projection is rendered as a status row or panel. Action
//! authority is represented only by an opaque capability issued by
//! `NotifyService`; no command or target crosses this DTO.
//!
//! ## Rendering contract
//!
//! Composition calls [`WlcontrolSkStatus::from_state`] for the in-process
//! observer projection. The client MUST NOT discover a state-file control
//! endpoint or embed ceremony lifecycle logic.
//!
//! ## Action routing
//!
//! Each [`WlcontrolAction`] carries an opaque capability. The client supplies
//! fresh request and idempotency identifiers and invokes
//! `NotifyService.InvokeAction` on its authenticated `ComponentSession`.
//! There is no CLI, file, or alternate socket fallback.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    services::actions::{
        ActionKind, ActionOffer, IDEMPOTENCY_KEY_BYTES, InvokeActionRequest, REQUEST_ID_BYTES,
    },
    state::{CeremonySummary, SkNotifyState},
};

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

/// A single action offer. Its capability is intentionally target-free.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WlcontrolAction {
    pub label: String,
    pub offer: ActionOffer,
}

impl std::fmt::Debug for WlcontrolAction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WlcontrolAction")
            .field("label", &self.label)
            .field("kind", &self.offer.kind)
            .field("capability", &"<redacted>")
            .finish()
    }
}

impl WlcontrolAction {
    pub fn from_offer(offer: ActionOffer) -> Self {
        let label = match offer.kind {
            ActionKind::CancelSecurityKeyCeremony => "Cancel request",
        };
        Self {
            label: label.to_owned(),
            offer,
        }
    }

    pub fn invocation(
        &self,
        request_id: [u8; REQUEST_ID_BYTES],
        idempotency_key: [u8; IDEMPOTENCY_KEY_BYTES],
        issued_at_unix_ms: u64,
        expires_at_unix_ms: u64,
    ) -> InvokeActionRequest {
        InvokeActionRequest::new(
            request_id,
            idempotency_key,
            self.offer.capability.clone(),
            issued_at_unix_ms,
            expires_at_unix_ms,
        )
    }
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
    /// `action_builder` is a trusted in-process callback that issues zero or
    /// more opaque service offers for each active ceremony.
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
    use crate::services::actions::{ActionService, ActionSession, EstablishedComponentSession};
    use crate::state::SkNotifyState;

    const T0: u64 = 1_750_000_000;

    struct Session;

    impl EstablishedComponentSession for Session {
        fn service_package(&self) -> &str {
            crate::services::actions::SERVICE_PACKAGE
        }
        fn endpoint_purpose(&self) -> &str {
            crate::services::actions::ENDPOINT_PURPOSE
        }
        fn endpoint_role(&self) -> &str {
            crate::services::actions::ENDPOINT_ROLE
        }
        fn is_authenticated(&self) -> bool {
            true
        }
        fn uses_pre_authorized_transport(&self) -> bool {
            true
        }
    }

    fn action_service() -> ActionService {
        ActionService::new(ActionSession::admit(&Session).unwrap())
    }

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
        let offer = action_service().offer_cancel("s1", T0).unwrap();
        let action = WlcontrolAction::from_offer(offer);
        let status = WlcontrolSkStatus::from_state(&state, |_| vec![action.clone()]);
        let json = serde_json::to_string(&status).unwrap();
        let decoded: WlcontrolSkStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.active[0].actions[0].label, "Cancel request");
        assert_eq!(
            decoded.active[0].actions[0].offer.capability,
            action.offer.capability
        );
        assert!(!json.contains("\"target\""));
        assert!(!json.contains("s1\",\"label\""));
    }

    #[test]
    fn invocation_contains_only_service_metadata_and_opaque_capability() {
        let offer = action_service().offer_cancel("private-target", T0).unwrap();
        let action = WlcontrolAction::from_offer(offer);
        let request = action.invocation([1; 16], [2; 16], T0 * 1_000, T0 * 1_000 + 1_000);
        let wire = serde_json::to_string(&request).unwrap();
        assert!(!wire.contains("private-target"));
        assert!(!wire.contains("cancel"));
        assert!(!format!("{action:?}").contains(action.offer.capability.expose()));
    }
}
