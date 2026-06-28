use std::time::{Duration, Instant};

use crate::niri::FocusedWindowSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FallbackState {
    Idle,
    PickerOpen {
        target: FocusedWindowSnapshot,
    },
    Armed {
        entry_id: String,
        target: FocusedWindowSnapshot,
        expires_at: Instant,
        ignore_target_restore: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackTransition {
    Idle,
    PickerOpen,
    Armed,
    Cleared(FallbackClearReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackClearReason {
    FocusChanged,
    TargetDisappeared,
    Timeout,
    NativeSelectionChanged,
    PickerCancelled,
}

#[derive(Debug, Clone)]
pub struct FallbackArming {
    state: FallbackState,
}

impl Default for FallbackArming {
    fn default() -> Self {
        Self {
            state: FallbackState::Idle,
        }
    }
}

impl FallbackArming {
    pub fn state(&self) -> &FallbackState {
        &self.state
    }

    pub fn capture_target_before_picker(
        &mut self,
        target: FocusedWindowSnapshot,
    ) -> FallbackTransition {
        self.state = FallbackState::PickerOpen { target };
        FallbackTransition::PickerOpen
    }

    pub fn arm_selected_entry(
        &mut self,
        entry_id: String,
        now: Instant,
        timeout: Duration,
    ) -> FallbackTransition {
        let FallbackState::PickerOpen { target } = &self.state else {
            return FallbackTransition::Idle;
        };
        self.state = FallbackState::Armed {
            entry_id,
            target: target.clone(),
            expires_at: now + timeout,
            ignore_target_restore: true,
        };
        FallbackTransition::Armed
    }

    pub fn cancel_picker(&mut self) -> FallbackTransition {
        if matches!(self.state, FallbackState::Idle) {
            return FallbackTransition::Idle;
        }
        self.state = FallbackState::Idle;
        FallbackTransition::Cleared(FallbackClearReason::PickerCancelled)
    }

    pub fn on_focus_changed(
        &mut self,
        focused: Option<FocusedWindowSnapshot>,
    ) -> FallbackTransition {
        let FallbackState::Armed {
            target,
            ignore_target_restore,
            ..
        } = &mut self.state
        else {
            return current_transition(&self.state);
        };
        let Some(focused) = focused else {
            self.state = FallbackState::Idle;
            return FallbackTransition::Cleared(FallbackClearReason::TargetDisappeared);
        };
        if target.same_target(&focused) {
            if *ignore_target_restore {
                *ignore_target_restore = false;
            }
            FallbackTransition::Armed
        } else {
            self.state = FallbackState::Idle;
            FallbackTransition::Cleared(FallbackClearReason::FocusChanged)
        }
    }

    pub fn on_timeout(&mut self, now: Instant) -> FallbackTransition {
        let FallbackState::Armed { expires_at, .. } = &self.state else {
            return current_transition(&self.state);
        };
        if now >= *expires_at {
            self.state = FallbackState::Idle;
            FallbackTransition::Cleared(FallbackClearReason::Timeout)
        } else {
            FallbackTransition::Armed
        }
    }

    pub fn on_native_selection_changed(&mut self) -> FallbackTransition {
        if matches!(self.state, FallbackState::Idle) {
            return FallbackTransition::Idle;
        }
        self.state = FallbackState::Idle;
        FallbackTransition::Cleared(FallbackClearReason::NativeSelectionChanged)
    }
}

fn current_transition(state: &FallbackState) -> FallbackTransition {
    match state {
        FallbackState::Idle => FallbackTransition::Idle,
        FallbackState::PickerOpen { .. } => FallbackTransition::PickerOpen,
        FallbackState::Armed { .. } => FallbackTransition::Armed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: u64, app_id: &str) -> FocusedWindowSnapshot {
        FocusedWindowSnapshot {
            id: Some(id),
            app_id: Some(app_id.to_owned()),
            title: Some("target".to_owned()),
            workspace_id: Some(1),
            output_label: Some("DP-1".to_owned()),
        }
    }

    #[test]
    fn captures_target_before_picker_and_arms_without_synthetic_input() {
        let mut arming = FallbackArming::default();
        let now = Instant::now();

        assert_eq!(
            arming.capture_target_before_picker(target(7, "firefox")),
            FallbackTransition::PickerOpen
        );
        assert_eq!(
            arming.arm_selected_entry("entry-a".to_owned(), now, Duration::from_secs(2)),
            FallbackTransition::Armed
        );

        assert!(matches!(
            arming.state(),
            FallbackState::Armed {
                entry_id,
                target,
                ..
            } if entry_id == "entry-a" && target.id == Some(7)
        ));
    }

    #[test]
    fn ignores_expected_picker_to_target_focus_restoration_once() {
        let mut arming = FallbackArming::default();
        let now = Instant::now();
        arming.capture_target_before_picker(target(7, "firefox"));
        arming.arm_selected_entry("entry-a".to_owned(), now, Duration::from_secs(2));

        assert_eq!(
            arming.on_focus_changed(Some(target(7, "firefox"))),
            FallbackTransition::Armed
        );
        assert!(matches!(
            arming.state(),
            FallbackState::Armed {
                ignore_target_restore: false,
                ..
            }
        ));
    }

    #[test]
    fn clears_on_unexpected_focus_change_or_disappearing_target() {
        let mut arming = FallbackArming::default();
        let now = Instant::now();
        arming.capture_target_before_picker(target(7, "firefox"));
        arming.arm_selected_entry("entry-a".to_owned(), now, Duration::from_secs(2));

        assert_eq!(
            arming.on_focus_changed(Some(target(8, "foot"))),
            FallbackTransition::Cleared(FallbackClearReason::FocusChanged)
        );
        assert_eq!(arming.state(), &FallbackState::Idle);

        arming.capture_target_before_picker(target(7, "firefox"));
        arming.arm_selected_entry("entry-a".to_owned(), now, Duration::from_secs(2));
        assert_eq!(
            arming.on_focus_changed(None),
            FallbackTransition::Cleared(FallbackClearReason::TargetDisappeared)
        );
    }

    #[test]
    fn clears_on_timeout_and_new_native_selection() {
        let mut arming = FallbackArming::default();
        let now = Instant::now();
        arming.capture_target_before_picker(target(7, "firefox"));
        arming.arm_selected_entry("entry-a".to_owned(), now, Duration::from_millis(5));

        assert_eq!(
            arming.on_timeout(now + Duration::from_millis(6)),
            FallbackTransition::Cleared(FallbackClearReason::Timeout)
        );

        arming.capture_target_before_picker(target(7, "firefox"));
        arming.arm_selected_entry("entry-a".to_owned(), now, Duration::from_secs(2));
        assert_eq!(
            arming.on_native_selection_changed(),
            FallbackTransition::Cleared(FallbackClearReason::NativeSelectionChanged)
        );
    }
}
