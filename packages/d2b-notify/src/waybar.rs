// SPDX-License-Identifier: Apache-2.0
//! Waybar JSON-protocol block helper.
//!
//! Waybar `custom/` module blocks receive JSON on stdin from a command.  The
//! expected shape is:
//!
//! ```json
//! {"text": "…", "tooltip": "…", "class": "…", "percentage": 0}
//! ```
//!
//! Only `text` is required; the rest are optional.  This module provides a
//! typed struct and a pure function that derives a `WaybarBlock` from the
//! current [`SkNotifyState`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::state::SkNotifyState;

/// JSON block emitted by the `d2b-sk-waybar-helper` binary to its Waybar
/// `custom/` module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WaybarBlock {
    /// Primary text shown in the bar.
    pub text: String,
    /// Tooltip shown on hover (newlines supported).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    /// CSS class applied to the widget (e.g. `"d2b-sk-idle"`,
    /// `"d2b-sk-active"`, `"d2b-sk-touch"`, `"d2b-sk-busy"`,
    /// `"d2b-sk-error"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
}

/// Produce a [`WaybarBlock`] from the current durable security-key state.
///
/// The block is:
/// - Idle (no active ceremonies): minimal icon, no tooltip, class `d2b-sk-idle`.
/// - One active ceremony in "started" or "touch-needed": icon + count, class
///   `d2b-sk-touch`.
/// - One or more active ceremonies in any other state: icon + count, class
///   `d2b-sk-active`.
/// - Error/busy: class `d2b-sk-busy`.
pub fn waybar_block_from_state(state: &SkNotifyState) -> WaybarBlock {
    if !state.has_active() {
        return WaybarBlock {
            text: "".to_owned(),
            tooltip: None,
            class: Some("d2b-sk-idle".to_owned()),
        };
    }

    let count = state.active.len();
    let has_touch = state.active.iter().any(|s| {
        s.last_event_kind == "touchNeeded" || s.last_event_kind == "started"
    });
    let has_busy = state.active.iter().any(|s| s.last_event_kind == "busy");

    let (icon, css_class) = if has_touch {
        ("🔑", "d2b-sk-touch")
    } else if has_busy {
        ("🔑", "d2b-sk-busy")
    } else {
        ("🔑", "d2b-sk-active")
    };

    let text = if count == 1 {
        format!("{icon}")
    } else {
        format!("{icon} {count}")
    };

    let tooltip_lines: Vec<String> = state
        .active
        .iter()
        .map(|s| {
            let kind_label = match s.last_event_kind.as_str() {
                "started" => "requesting",
                "touchNeeded" => "needs touch",
                "busy" => "waiting (busy)",
                "queued" => "queued",
                "blocked" => "blocked",
                other => other,
            };
            format!("{}: {kind_label}", s.vm_name)
        })
        .collect();

    WaybarBlock {
        text,
        tooltip: Some(tooltip_lines.join("\n")),
        class: Some(css_class.to_owned()),
    }
}

/// Emit a [`WaybarBlock`] to stdout as a JSON line (the format expected by
/// Waybar `custom/` with `return-type = "json"`).
pub fn print_waybar_block(block: &WaybarBlock) -> serde_json::Result<()> {
    let line = serde_json::to_string(block)?;
    println!("{line}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::SecurityKeyEvent;
    use crate::state::SkNotifyState;

    const T0: u64 = 1_750_000_000;

    #[test]
    fn idle_state_produces_idle_block() {
        let state = SkNotifyState::empty(T0);
        let block = waybar_block_from_state(&state);
        assert_eq!(block.class.as_deref(), Some("d2b-sk-idle"));
        assert!(block.tooltip.is_none());
    }

    #[test]
    fn started_ceremony_produces_touch_class() {
        let state = SkNotifyState::empty(T0).apply(
            &SecurityKeyEvent::Started {
                session_id: "s1".to_owned(),
                vm_name: "personal-dev".to_owned(),
                rp_id: None,
            },
            T0,
        );
        let block = waybar_block_from_state(&state);
        assert_eq!(block.class.as_deref(), Some("d2b-sk-touch"));
        assert!(block.text.contains('🔑'));
    }

    #[test]
    fn busy_ceremony_produces_busy_class() {
        use crate::events::BusyDetail;
        let state = SkNotifyState::empty(T0).apply(
            &SecurityKeyEvent::Busy {
                session_id: "s1".to_owned(),
                vm_name: "work-aad".to_owned(),
                detail: BusyDetail {
                    holder_vm: "personal-dev".to_owned(),
                    waiting_vms: vec![],
                },
            },
            T0,
        );
        let block = waybar_block_from_state(&state);
        assert_eq!(block.class.as_deref(), Some("d2b-sk-busy"));
    }

    #[test]
    fn multiple_active_ceremonies_shows_count() {
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
        let block = waybar_block_from_state(&state);
        assert!(block.text.contains('2'), "count should appear for 2 active ceremonies");
    }

    #[test]
    fn tooltip_lists_active_vm_names() {
        let state = SkNotifyState::empty(T0)
            .apply(
                &SecurityKeyEvent::Started {
                    session_id: "s1".to_owned(),
                    vm_name: "personal-dev".to_owned(),
                    rp_id: None,
                },
                T0,
            );
        let block = waybar_block_from_state(&state);
        let tooltip = block.tooltip.unwrap();
        assert!(tooltip.contains("personal-dev"));
    }

    #[test]
    fn waybar_block_serializes_to_json() {
        let block = WaybarBlock {
            text: "🔑".to_owned(),
            tooltip: Some("vm: needs touch".to_owned()),
            class: Some("d2b-sk-touch".to_owned()),
        };
        let json = serde_json::to_string(&block).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["text"], "🔑");
        assert_eq!(parsed["class"], "d2b-sk-touch");
    }
}
