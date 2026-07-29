//! The actions a tab can offer.
//!
//! Names describe the outcome the operator gets, not the subsystem that
//! implements it, and each action declares whether it is destructive or opens
//! further controls. Those two flags are what stop a dispatcher from treating
//! `stop-vm` as an ordinary one-release activation.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Terminal,
    Audio,
    Usb,
    Info,
    Stop,
}

impl Action {
    pub const DEFAULTS: [Action; 5] = [
        Action::Terminal,
        Action::Audio,
        Action::Usb,
        Action::Info,
        Action::Stop,
    ];

    /// Stable identifier used in config, dispatch and logs.
    pub fn name(&self) -> &'static str {
        match self {
            Action::Terminal => "open-terminal",
            Action::Audio => "audio-controls",
            Action::Usb => "usb-devices",
            Action::Info => "vm-details",
            Action::Stop => "stop-vm",
        }
    }

    /// Short label drawn beside the icon. Five bare glyphs proved semantically
    /// ambiguous, so the icon accelerates a label rather than replacing it.
    pub fn label(&self) -> &'static str {
        match self {
            Action::Terminal => "Open terminal",
            Action::Audio => "Audio…",
            Action::Usb => "USB devices…",
            Action::Info => "VM details…",
            Action::Stop => "Stop VM…",
        }
    }

    /// Whether activating this action is destructive, and therefore needs an
    /// explicit confirmation rather than a single release.
    pub fn is_destructive(&self) -> bool {
        matches!(self, Action::Stop)
    }

    /// Whether this action opens further controls rather than acting
    /// immediately. These need a second disclosure tier, not a toggle: a
    /// volume level and a USB device list cannot be expressed as one icon.
    pub fn opens_submenu(&self) -> bool {
        matches!(self, Action::Audio | Action::Usb | Action::Info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_outcome_shaped() {
        let mut seen = Vec::new();
        for a in Action::DEFAULTS {
            assert!(!seen.contains(&a.name()), "duplicate name {}", a.name());
            assert!(
                a.name().contains('-'),
                "`{}` should read as an outcome",
                a.name()
            );
            seen.push(a.name());
        }
    }

    #[test]
    fn every_action_has_a_visible_label() {
        for a in Action::DEFAULTS {
            assert!(!a.label().is_empty());
        }
    }

    #[test]
    fn exactly_one_default_action_is_destructive() {
        let destructive: Vec<_> = Action::DEFAULTS
            .iter()
            .filter(|a| a.is_destructive())
            .collect();
        assert_eq!(destructive, vec![&Action::Stop]);
    }

    #[test]
    fn actions_needing_richer_controls_declare_it() {
        // Audio level, USB device list and VM details cannot be a single
        // toggle, so they must be marked as opening a second tier.
        for a in [Action::Audio, Action::Usb, Action::Info] {
            assert!(a.opens_submenu(), "{} must open a submenu", a.name());
        }
        assert!(!Action::Terminal.opens_submenu());
        assert!(!Action::Stop.opens_submenu());
    }

    #[test]
    fn a_destructive_action_never_also_opens_a_submenu() {
        // Otherwise it is ambiguous whether a release confirms or discloses.
        for a in Action::DEFAULTS {
            assert!(
                !(a.is_destructive() && a.opens_submenu()),
                "{} is both destructive and disclosing",
                a.name()
            );
        }
    }

    #[test]
    fn names_round_trip_through_serde() {
        for a in Action::DEFAULTS {
            let json = serde_json::to_string(&a).unwrap();
            let back: Action = serde_json::from_str(&json).unwrap();
            assert_eq!(a, back);
        }
    }
}
