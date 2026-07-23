// SPDX-License-Identifier: Apache-2.0
//! Typed security-key event enum.
//!
//! Events are emitted by the host security-key broker and consumed by the
//! desktop notification layer. Each variant corresponds to one stage in the
//! CTAP/WebAuthn ceremony lifecycle.
//!
//! Consumers (notification forwarder, Waybar helper, wlcontrol) receive these
//! via the durable state file ([`crate::state`]) and must not perform any
//! privileged host mutations in response — all callbacks route through
//! `d2bd`/CLI via action nonces ([`crate::nonce`]).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Phase in a security-key ceremony at which the key is occupied by a
/// concurrent request from a different VM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BusyDetail {
    /// VM name that currently holds the active ceremony lease.
    pub holder_vm: String,
    /// VMs waiting behind the active holder (may be empty).
    #[serde(default)]
    pub waiting_vms: Vec<String>,
}

/// Why a security-key ceremony was blocked before it could start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum BlockReason {
    /// The physical security key is not present or was removed.
    KeyNotPresent,
    /// Policy denies this VM's access to the security key.
    PolicyDenied,
    /// The broker received a request that does not match any declared VM opt-in.
    VmNotOptedIn,
    /// A transport or internal broker error prevented the ceremony.
    BrokerError,
}

/// Typed event emitted by the host security-key broker for one CTAP/WebAuthn
/// ceremony attempt.
///
/// Each event carries the opaque `session_id` that identifies the specific
/// ceremony instance plus the human-readable `vm_name` of the requesting VM.
/// The `session_id` is also the binding anchor for action nonces
/// ([`crate::nonce`]) so that `Cancel request` callbacks cannot be replayed
/// against a different session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum SecurityKeyEvent {
    /// A VM requested a CTAP/WebAuthn ceremony and the broker accepted the
    /// lease. This is the initial event for a new ceremony session.
    #[serde(rename_all = "camelCase")]
    Started {
        /// Opaque ceremony session identifier (hex nonce).
        session_id: String,
        /// VM that initiated the ceremony.
        vm_name: String,
        /// Relying-party identifier extracted from the CTAP request if the
        /// proxy is command-aware; `None` when the proxy operates in pass-
        /// through mode.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rp_id: Option<String>,
    },

    /// The browser is waiting for a physical user-presence touch. Emitted
    /// when the CTAP layer transitions to the UP-required state; only
    /// available when the proxy is command-aware.
    #[serde(rename_all = "camelCase")]
    TouchNeeded { session_id: String, vm_name: String },

    /// Another VM currently holds the security-key lease; this session is
    /// either waiting in the queue or was bumped.
    #[serde(rename_all = "camelCase")]
    Busy {
        session_id: String,
        vm_name: String,
        /// Detail about the current holder and queue.
        detail: BusyDetail,
    },

    /// This session is waiting in the broker queue behind one or more active
    /// or higher-priority requests.
    #[serde(rename_all = "camelCase")]
    Queued {
        session_id: String,
        vm_name: String,
        /// Position in the wait queue (1-based).
        queue_position: u32,
    },

    /// A policy or hardware condition prevents this ceremony from proceeding
    /// before it started.
    #[serde(rename_all = "camelCase")]
    Blocked {
        session_id: String,
        vm_name: String,
        reason: BlockReason,
    },

    /// The ceremony reached the configured timeout without a successful touch.
    #[serde(rename_all = "camelCase")]
    TimedOut { session_id: String, vm_name: String },

    /// The ceremony completed with a non-timeout failure.
    #[serde(rename_all = "camelCase")]
    Failed {
        session_id: String,
        vm_name: String,
        /// Human-readable reason; must not leak secrets or internal paths.
        reason: String,
    },

    /// The ceremony was explicitly canceled via `d2b usb security-key cancel`
    /// or the `Cancel request` notification action.
    #[serde(rename_all = "camelCase")]
    Canceled { session_id: String, vm_name: String },

    /// A previously active ceremony completed successfully.
    #[serde(rename_all = "camelCase")]
    Completed { session_id: String, vm_name: String },
}

impl SecurityKeyEvent {
    /// Opaque session identifier shared by all event variants.
    pub fn session_id(&self) -> &str {
        match self {
            Self::Started { session_id, .. }
            | Self::TouchNeeded { session_id, .. }
            | Self::Busy { session_id, .. }
            | Self::Queued { session_id, .. }
            | Self::Blocked { session_id, .. }
            | Self::TimedOut { session_id, .. }
            | Self::Failed { session_id, .. }
            | Self::Canceled { session_id, .. }
            | Self::Completed { session_id, .. } => session_id,
        }
    }

    /// VM name associated with this event.
    pub fn vm_name(&self) -> &str {
        match self {
            Self::Started { vm_name, .. }
            | Self::TouchNeeded { vm_name, .. }
            | Self::Busy { vm_name, .. }
            | Self::Queued { vm_name, .. }
            | Self::Blocked { vm_name, .. }
            | Self::TimedOut { vm_name, .. }
            | Self::Failed { vm_name, .. }
            | Self::Canceled { vm_name, .. }
            | Self::Completed { vm_name, .. } => vm_name,
        }
    }

    /// Whether this event represents a terminal state (no further events
    /// expected for this `session_id`).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::TimedOut { .. }
                | Self::Failed { .. }
                | Self::Canceled { .. }
                | Self::Completed { .. }
        )
    }

    /// Whether this event requires user attention (should trigger a desktop
    /// notification).
    pub fn is_user_visible(&self) -> bool {
        matches!(
            self,
            Self::Started { .. }
                | Self::TouchNeeded { .. }
                | Self::Busy { .. }
                | Self::Blocked { .. }
                | Self::TimedOut { .. }
                | Self::Failed { .. }
                | Self::Canceled { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn started_event_round_trips_without_rp_id() {
        let event = SecurityKeyEvent::Started {
            session_id: "abc123".to_owned(),
            vm_name: "personal-dev".to_owned(),
            rp_id: None,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["kind"], "started");
        assert_eq!(json["sessionId"], "abc123");
        assert_eq!(json["vmName"], "personal-dev");
        assert!(json.get("rpId").is_none(), "rpId must be omitted when None");

        let decoded: SecurityKeyEvent = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, event);
    }

    #[test]
    fn started_event_round_trips_with_rp_id() {
        let event = SecurityKeyEvent::Started {
            session_id: "abc123".to_owned(),
            vm_name: "work-aad".to_owned(),
            rp_id: Some("github.com".to_owned()),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["rpId"], "github.com");
        let decoded: SecurityKeyEvent = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, event);
    }

    #[test]
    fn touch_needed_is_user_visible() {
        let event = SecurityKeyEvent::TouchNeeded {
            session_id: "s1".to_owned(),
            vm_name: "vm1".to_owned(),
        };
        assert!(event.is_user_visible());
        assert!(!event.is_terminal());
    }

    #[test]
    fn timed_out_is_terminal_and_user_visible() {
        let event = SecurityKeyEvent::TimedOut {
            session_id: "s1".to_owned(),
            vm_name: "vm1".to_owned(),
        };
        assert!(event.is_terminal());
        assert!(event.is_user_visible());
    }

    #[test]
    fn completed_is_terminal_not_user_visible() {
        let event = SecurityKeyEvent::Completed {
            session_id: "s1".to_owned(),
            vm_name: "vm1".to_owned(),
        };
        assert!(event.is_terminal());
        assert!(!event.is_user_visible());
    }

    #[test]
    fn busy_event_round_trips() {
        let event = SecurityKeyEvent::Busy {
            session_id: "s2".to_owned(),
            vm_name: "work-aad".to_owned(),
            detail: BusyDetail {
                holder_vm: "personal-dev".to_owned(),
                waiting_vms: vec![],
            },
        };
        let json_val = serde_json::to_value(&event).unwrap();
        assert_eq!(json_val["kind"], "busy");
        assert_eq!(json_val["detail"]["holderVm"], "personal-dev");
        let decoded: SecurityKeyEvent = serde_json::from_value(json_val).unwrap();
        assert_eq!(decoded, event);
    }

    #[test]
    fn unknown_event_kind_rejected() {
        let json_val = json!({"kind": "zapped", "sessionId": "s1", "vmName": "vm1"});
        let result = serde_json::from_value::<SecurityKeyEvent>(json_val);
        assert!(result.is_err(), "unknown kind must be rejected");
    }

    #[test]
    fn session_id_accessor_works_for_all_variants() {
        let variants: Vec<SecurityKeyEvent> = vec![
            SecurityKeyEvent::Started {
                session_id: "sid".to_owned(),
                vm_name: "v".to_owned(),
                rp_id: None,
            },
            SecurityKeyEvent::TouchNeeded {
                session_id: "sid".to_owned(),
                vm_name: "v".to_owned(),
            },
            SecurityKeyEvent::TimedOut {
                session_id: "sid".to_owned(),
                vm_name: "v".to_owned(),
            },
            SecurityKeyEvent::Canceled {
                session_id: "sid".to_owned(),
                vm_name: "v".to_owned(),
            },
            SecurityKeyEvent::Completed {
                session_id: "sid".to_owned(),
                vm_name: "v".to_owned(),
            },
        ];
        for v in &variants {
            assert_eq!(v.session_id(), "sid");
            assert_eq!(v.vm_name(), "v");
        }
    }
}
