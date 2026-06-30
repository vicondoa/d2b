//! Rate-limited, payload-free diagnostics.
//!
//! All log events must contain only bounded metadata: VM name, interface name,
//! action, reason code, and numeric registry name. Never log titles, clipboard
//! payloads, DnD data, raw app-id values, or message bodies.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// Bounded reason code for client-drop events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DropReason {
    BindDeniedUnadvertised,
    BindDeniedHidden,
}

impl DropReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::BindDeniedUnadvertised => "bind-denied-unadvertised",
            Self::BindDeniedHidden => "bind-denied-hidden",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RateKey {
    vm: String,
    event: &'static str,
    label: String,
}

#[derive(Debug)]
struct BucketState {
    count: u64,
    window_start: Instant,
    suppressed: u64,
}

const WINDOW: Duration = Duration::from_secs(60);
const MAX_PER_WINDOW: u64 = 5;
const DIAG_ERROR_MAX_BYTES: usize = 256;
const DIAG_LABEL_MAX_BYTES: usize = 64;
const MAX_BUCKETS: usize = 256;
const OVERFLOW_LABEL: &str = "__overflow__";

fn bounded_diag_label(label: &str) -> String {
    let mut out = String::new();
    for ch in label.chars() {
        if out.len() + ch.len_utf8() > DIAG_LABEL_MAX_BYTES {
            out.push_str("...");
            break;
        }
        if ch.is_ascii_graphic() || ch == ' ' {
            out.push(ch);
        } else {
            out.push('?');
        }
    }
    out
}

pub fn bounded_error_detail(error: impl Into<String>) -> String {
    let error = error
        .into()
        .chars()
        .map(|ch| {
            if ch.is_control() && ch != '\t' {
                '?'
            } else {
                ch
            }
        })
        .collect::<String>();
    if error.len() <= DIAG_ERROR_MAX_BYTES {
        return error;
    }
    let mut end = DIAG_ERROR_MAX_BYTES;
    while !error.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &error[..end])
}

/// Per-state rate limiter for filter diagnostics.
#[derive(Debug)]
pub struct DiagRateLimiter {
    vm: String,
    buckets: HashMap<RateKey, BucketState>,
}

impl DiagRateLimiter {
    pub fn new(vm: String) -> Self {
        Self {
            vm,
            buckets: HashMap::new(),
        }
    }

    /// Emit a rate-limited log at WARN level.
    /// Returns `true` if the event was emitted (not suppressed), `false` if rate-limited.
    fn emit(&mut self, event: &'static str, label: &str, msg: impl FnOnce() -> String) -> bool {
        let now = Instant::now();
        let bounded_label = bounded_diag_label(label);
        self.prune_expired(now);
        let label = if self.buckets.len() >= MAX_BUCKETS
            && !self.buckets.contains_key(&RateKey {
                vm: self.vm.clone(),
                event,
                label: bounded_label.clone(),
            }) {
            OVERFLOW_LABEL.to_owned()
        } else {
            bounded_label
        };
        let key = RateKey {
            vm: self.vm.clone(),
            event,
            label: label.clone(),
        };
        let bucket = self.buckets.entry(key).or_insert_with(|| BucketState {
            count: 0,
            window_start: now,
            suppressed: 0,
        });

        if now.duration_since(bucket.window_start) >= WINDOW {
            // Flush suppressed count if any.
            if bucket.suppressed > 0 {
                log::warn!(
                    "[d2b-wlproxy] vm={} event={event} label={label} \
                     suppressed={} in last window",
                    self.vm,
                    bucket.suppressed,
                );
            }

            bucket.count = 0;
            bucket.suppressed = 0;
            bucket.window_start = now;
        }

        if bucket.count < MAX_PER_WINDOW {
            bucket.count += 1;
            log::warn!("{}", msg());
            true
        } else {
            bucket.suppressed += 1;
            false
        }
    }

    fn prune_expired(&mut self, now: Instant) {
        self.buckets.retain(|_, bucket| {
            bucket.suppressed > 0 || now.duration_since(bucket.window_start) < WINDOW
        });
    }

    pub fn warn(&mut self, event: &'static str, label: &str, msg: impl FnOnce() -> String) -> bool {
        self.emit(event, label, msg)
    }

    /// Log a bind-denial event (security boundary enforcement).
    /// Always emits via `log::warn`; rate-limited per (vm, event, interface).
    pub fn bind_denied(&mut self, reason: DropReason, registry_name: u32, interface: &str) {
        let reason_str = reason.as_str();
        let vm = self.vm.clone();
        let interface = bounded_diag_label(interface);
        self.emit("bind-denied", &interface, || {
            format!(
                "[d2b-wlproxy] vm={vm} event=bind-denied reason={reason_str} \
                 interface={interface} registry-name={registry_name}"
            )
        });
    }

    /// Log a global-filtered event (advertisement filtered; opt-in via `--log-filtered-globals`).
    pub fn global_filtered(&mut self, interface: &str) {
        let vm = self.vm.clone();
        let interface = bounded_diag_label(interface);
        self.emit("global-filtered", &interface, || {
            format!("[d2b-wlproxy] vm={vm} event=global-filtered interface={interface}")
        });
    }

    /// Flush suppressed counts for all buckets. Call periodically and before
    /// shutdown so a terminal burst does not disappear just because no later
    /// event arrived after the rate-limit window.
    pub fn flush_suppressed(&mut self) {
        let now = Instant::now();
        for (key, bucket) in &mut self.buckets {
            if bucket.suppressed == 0 {
                continue;
            }
            log::warn!(
                "[d2b-wlproxy] vm={} event={} label={} suppressed={} in last window",
                key.vm,
                key.event,
                key.label,
                bucket.suppressed,
            );
            bucket.suppressed = 0;
        }
        self.prune_expired(now);
    }

    #[cfg(test)]
    pub fn suppressed_total_for_tests(&self) -> u64 {
        self.buckets.values().map(|bucket| bucket.suppressed).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limiter_suppresses_after_max() {
        let mut rl = DiagRateLimiter::new("test-vm".to_owned());
        // Emit MAX_PER_WINDOW events for the same key — all should pass.
        for _ in 0..MAX_PER_WINDOW {
            let emitted = rl.emit("test-event", "test-label", || "msg".to_owned());
            assert!(emitted);
        }
        // The next one must be suppressed.
        let emitted = rl.emit("test-event", "test-label", || "msg".to_owned());
        assert!(!emitted);
    }

    #[test]
    fn different_labels_have_independent_buckets() {
        let mut rl = DiagRateLimiter::new("vm".to_owned());
        for _ in 0..MAX_PER_WINDOW {
            rl.emit("ev", "a", || "msg".to_owned());
        }
        // "b" bucket is fresh.
        let emitted = rl.emit("ev", "b", || "msg".to_owned());
        assert!(emitted);
    }

    #[test]
    fn flush_suppressed_resets_terminal_burst_counts() {
        let mut rl = DiagRateLimiter::new("vm".to_owned());
        for _ in 0..MAX_PER_WINDOW {
            assert!(rl.emit("ev", "a", || "msg".to_owned()));
        }
        assert!(!rl.emit("ev", "a", || "msg".to_owned()));

        let bucket = rl.buckets.values().next().expect("bucket exists");
        assert_eq!(bucket.suppressed, 1);

        rl.flush_suppressed();

        let bucket = rl.buckets.values().next().expect("bucket exists");
        assert_eq!(bucket.suppressed, 0);
    }

    #[test]
    fn bounded_error_detail_truncates_on_utf8_boundary() {
        let detail = format!("{}{}", "a".repeat(DIAG_ERROR_MAX_BYTES - 1), "é".repeat(4));
        let bounded = bounded_error_detail(detail);
        assert!(bounded.len() <= DIAG_ERROR_MAX_BYTES + 3);
        assert!(bounded.ends_with("..."));
        assert!(std::str::from_utf8(bounded.as_bytes()).is_ok());
    }

    #[test]
    fn bounded_error_detail_preserves_short_errors() {
        assert_eq!(bounded_error_detail("short"), "short");
    }

    #[test]
    fn bounded_error_detail_scrubs_control_characters() {
        assert_eq!(bounded_error_detail("line1\nline2\r\0"), "line1?line2??");
    }

    #[test]
    fn bind_denied_rate_limits_by_interface() {
        let mut rl = DiagRateLimiter::new("vm".to_owned());
        for name in 0..MAX_PER_WINDOW as u32 {
            rl.bind_denied(
                DropReason::BindDeniedUnadvertised,
                name,
                "zwp_text_input_manager_v3",
            );
        }
        rl.bind_denied(
            DropReason::BindDeniedUnadvertised,
            100,
            "zwp_text_input_manager_v3",
        );
        assert_eq!(rl.suppressed_total_for_tests(), 1);

        rl.bind_denied(
            DropReason::BindDeniedUnadvertised,
            101,
            "wl_data_device_manager",
        );
        assert_eq!(rl.suppressed_total_for_tests(), 1);
    }

    #[test]
    fn rate_limiter_bounds_guest_controlled_label_cardinality() {
        let mut rl = DiagRateLimiter::new("test-vm".to_owned());
        for index in 0..(MAX_BUCKETS + 64) {
            let label = format!("guest-controlled-interface-{index}");
            let _ = rl.warn("bind-denied", &label, || "bounded".to_owned());
        }

        assert!(rl.buckets.len() <= MAX_BUCKETS + 1);
        assert!(rl.buckets.keys().any(|key| key.label == OVERFLOW_LABEL));
    }

    #[test]
    fn diagnostic_labels_are_bounded_and_control_free() {
        let label = bounded_diag_label(&format!("{}{}", "x".repeat(128), "\nsecret"));
        assert!(label.len() <= DIAG_LABEL_MAX_BYTES + "...".len());
        assert!(!label.contains('\n'));
    }
}
