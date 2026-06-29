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
        let key = RateKey {
            vm: self.vm.clone(),
            event,
            label: label.to_owned(),
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

    pub fn warn(&mut self, event: &'static str, label: &str, msg: impl FnOnce() -> String) -> bool {
        self.emit(event, label, msg)
    }

    /// Log a bind-denial event (security boundary enforcement).
    /// Always emits via `log::warn`; rate-limited per (vm, event, interface).
    pub fn bind_denied(&mut self, reason: DropReason, registry_name: u32) {
        let reason_str = reason.as_str();
        let vm = self.vm.clone();
        self.emit("bind-denied", reason_str, || {
            format!(
                "[d2b-wlproxy] vm={vm} event=bind-denied reason={reason_str} \
                 registry-name={registry_name}"
            )
        });
    }

    /// Log a global-filtered event (advertisement filtered; opt-in via `--log-filtered-globals`).
    pub fn global_filtered(&mut self, interface: &str) {
        let vm = self.vm.clone();
        let iface = interface.to_owned();
        self.emit("global-filtered", interface, || {
            format!("[d2b-wlproxy] vm={vm} event=global-filtered interface={iface}")
        });
    }

    /// Flush suppressed counts for all buckets. Call periodically and before
    /// shutdown so a terminal burst does not disappear just because no later
    /// event arrived after the rate-limit window.
    pub fn flush_suppressed(&mut self) {
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
}
