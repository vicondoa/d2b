//! Bounded, timestamped history rings.
//!
//! Three rings, because the "what changed" question has three different honest
//! answers depending on what the child is doing:
//!
//! * **dirty rows** -- which viewport rows `avt` marked changed. Cheap and
//!   exact for a full-screen TUI, where the viewport is fixed.
//! * **view checkpoints** -- periodic snapshots of the rendered viewport, so a
//!   real line diff can be computed against any point in the window. This is
//!   the answer that survives scrolling, where a dirty row index means "content
//!   moved through here" rather than "this content is new".
//! * **output log** -- decoded child output with timestamps, in asciicast v3
//!   event shape. The transcript of what actually happened, including rows that
//!   have since scrolled out of the viewport entirely.
//!
//! Every ring is bounded. An agent-facing tool that grows without limit while
//! watching a chatty build is a memory leak with extra steps.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::screen::CursorPos;

/// Default interval between view checkpoints.
pub const DEFAULT_CHECKPOINT_INTERVAL: Duration = Duration::from_millis(500);
/// Default number of view checkpoints retained.
pub const DEFAULT_CHECKPOINT_CAPACITY: usize = 240;
/// Default number of dirty-row records retained.
pub const DEFAULT_DIRTY_CAPACITY: usize = 4096;
/// Default number of output records retained.
pub const DEFAULT_OUTPUT_CAPACITY: usize = 4096;
/// Default cap on total bytes retained in the output ring.
pub const DEFAULT_OUTPUT_BYTES: usize = 1 << 20;
/// Default number of evicted scrollback lines retained.
pub const DEFAULT_SCROLLBACK_CAPACITY: usize = 4096;

/// Rows marked dirty by one feed.
#[derive(Debug, Clone)]
pub struct DirtyRecord {
    pub at: Instant,
    pub rows: Vec<usize>,
}

/// A rendered viewport at a point in time.
#[derive(Debug, Clone)]
pub struct Checkpoint {
    pub at: Instant,
    pub view: Vec<String>,
    pub alt_screen: bool,
}

/// One chunk of decoded child output.
#[derive(Debug, Clone)]
pub struct OutputRecord {
    pub at: Instant,
    pub text: String,
}

/// A line evicted from the emulator's scrollback.
#[derive(Debug, Clone)]
pub struct ScrollbackRecord {
    pub at: Instant,
    pub text: String,
}

/// Where the cursor was after a feed.
///
/// Tracked separately from dirty rows because `avt` deliberately does not mark
/// a line dirty for pure cursor movement. An application that repositions the
/// cursor on a timer therefore produces PTY traffic and zero dirty rows, and
/// distinguishing that from a genuinely changing screen is what makes idle
/// detection trustworthy.
#[derive(Debug, Clone, Copy)]
pub struct CursorRecord {
    pub at: Instant,
    pub cursor: CursorPos,
}

/// Tunable ring sizes.
#[derive(Debug, Clone, Copy)]
pub struct HistoryLimits {
    pub checkpoint_interval: Duration,
    pub checkpoint_capacity: usize,
    pub dirty_capacity: usize,
    pub output_capacity: usize,
    pub output_bytes: usize,
    pub scrollback_capacity: usize,
}

impl Default for HistoryLimits {
    fn default() -> Self {
        Self {
            checkpoint_interval: DEFAULT_CHECKPOINT_INTERVAL,
            checkpoint_capacity: DEFAULT_CHECKPOINT_CAPACITY,
            dirty_capacity: DEFAULT_DIRTY_CAPACITY,
            output_capacity: DEFAULT_OUTPUT_CAPACITY,
            output_bytes: DEFAULT_OUTPUT_BYTES,
            scrollback_capacity: DEFAULT_SCROLLBACK_CAPACITY,
        }
    }
}

/// The three rings plus their eviction bookkeeping.
#[derive(Debug)]
pub struct History {
    limits: HistoryLimits,
    dirty: VecDeque<DirtyRecord>,
    checkpoints: VecDeque<Checkpoint>,
    output: VecDeque<OutputRecord>,
    scrollback: VecDeque<ScrollbackRecord>,
    cursor: VecDeque<CursorRecord>,
    output_bytes: usize,
    last_checkpoint: Option<Instant>,
    /// Set once any ring has evicted, so a query can say so rather than
    /// silently reporting a partial answer as complete.
    evicted: bool,
}

impl History {
    pub fn new(limits: HistoryLimits) -> Self {
        Self {
            limits,
            dirty: VecDeque::new(),
            checkpoints: VecDeque::new(),
            output: VecDeque::new(),
            scrollback: VecDeque::new(),
            cursor: VecDeque::new(),
            output_bytes: 0,
            last_checkpoint: None,
            evicted: false,
        }
    }

    pub fn limits(&self) -> HistoryLimits {
        self.limits
    }

    pub fn has_evicted(&self) -> bool {
        self.evicted
    }

    /// Record the rows a feed marked dirty.
    ///
    /// Empty sets are dropped rather than stored: a feed that changed no rows (a
    /// pure mode change, say) would otherwise fill the ring with noise and trigger
    /// eviction of records that matter.
    pub fn record_dirty(&mut self, at: Instant, rows: Vec<usize>) {
        if rows.is_empty() {
            return;
        }
        self.dirty.push_back(DirtyRecord { at, rows });
        while self.dirty.len() > self.limits.dirty_capacity {
            self.dirty.pop_front();
            self.evicted = true;
        }
    }

    /// Record a chunk of decoded output.
    ///
    /// Bounded by record count *and* total bytes. The byte bound is the one that
    /// matters in practice: a full-screen TUI redrawing at 60 Hz produces few
    /// records but a great many bytes.
    pub fn record_output(&mut self, at: Instant, text: String) {
        if text.is_empty() {
            return;
        }
        self.output_bytes += text.len();
        self.output.push_back(OutputRecord { at, text });

        while self.output.len() > self.limits.output_capacity
            || self.output_bytes > self.limits.output_bytes
        {
            match self.output.pop_front() {
                Some(dropped) => {
                    self.output_bytes = self.output_bytes.saturating_sub(dropped.text.len());
                    self.evicted = true;
                }
                None => break,
            }
        }
    }

    /// Record lines the emulator evicted from its scrollback.
    pub fn record_scrollback(&mut self, at: Instant, lines: impl IntoIterator<Item = String>) {
        for text in lines {
            self.scrollback.push_back(ScrollbackRecord { at, text });
        }
        while self.scrollback.len() > self.limits.scrollback_capacity {
            self.scrollback.pop_front();
            self.evicted = true;
        }
    }

    /// Take a view checkpoint if the interval has elapsed.
    ///
    /// `force` bypasses the interval, which the session uses on resize and on
    /// an alternate-buffer switch: both invalidate row-index comparison, so a
    /// fresh baseline must exist at that instant.
    pub fn maybe_checkpoint(
        &mut self,
        at: Instant,
        force: bool,
        view: impl FnOnce() -> Vec<String>,
        alt_screen: bool,
    ) {
        let due = match self.last_checkpoint {
            None => true,
            Some(last) => at.duration_since(last) >= self.limits.checkpoint_interval,
        };

        if !due && !force {
            return;
        }

        self.checkpoints.push_back(Checkpoint {
            at,
            view: view(),
            alt_screen,
        });
        self.last_checkpoint = Some(at);

        while self.checkpoints.len() > self.limits.checkpoint_capacity {
            self.checkpoints.pop_front();
            self.evicted = true;
        }
    }

    /// The most recent checkpoint at or before `cutoff`.
    ///
    /// Searches backwards because the newest matching checkpoint is the tightest
    /// baseline, and recent queries are the common case.
    ///
    /// Returns `None` when the window reaches back further than any retained
    /// checkpoint, which the caller must report rather than paper over.
    pub fn checkpoint_at_or_before(&self, cutoff: Instant) -> Option<&Checkpoint> {
        self.checkpoints
            .iter()
            .rev()
            .find(|checkpoint| checkpoint.at <= cutoff)
    }

    /// The oldest retained checkpoint.
    pub fn oldest_checkpoint(&self) -> Option<&Checkpoint> {
        self.checkpoints.front()
    }

    /// Record where the cursor ended up after a feed.
    pub fn record_cursor(&mut self, at: Instant, cursor: CursorPos) {
        if self.cursor.back().is_some_and(|last| last.cursor == cursor) {
            // Unchanged; no need to store a duplicate.
            return;
        }
        self.cursor.push_back(CursorRecord { at, cursor });
        while self.cursor.len() > self.limits.dirty_capacity {
            self.cursor.pop_front();
            self.evicted = true;
        }
    }

    /// Whether the cursor moved at or after `cutoff`.
    pub fn cursor_moved_since(&self, cutoff: Instant) -> bool {
        self.cursor.iter().any(|record| record.at >= cutoff)
    }

    /// Union of every dirty row recorded at or after `cutoff`, sorted.
    ///
    /// Deduplicated, because a row touched on every frame of a redraw should be
    /// reported once, not once per frame.
    pub fn dirty_rows_since(&self, cutoff: Instant) -> Vec<usize> {
        let mut rows: Vec<usize> = self
            .dirty
            .iter()
            .filter(|record| record.at >= cutoff)
            .flat_map(|record| record.rows.iter().copied())
            .collect();
        rows.sort_unstable();
        rows.dedup();
        rows
    }

    /// Decoded output recorded at or after `cutoff`, concatenated.
    pub fn output_since(&self, cutoff: Instant) -> String {
        self.output
            .iter()
            .filter(|record| record.at >= cutoff)
            .map(|record| record.text.as_str())
            .collect()
    }

    /// Scrollback lines evicted at or after `cutoff`.
    pub fn scrollback_since(&self, cutoff: Instant) -> Vec<String> {
        self.scrollback
            .iter()
            .filter(|record| record.at >= cutoff)
            .map(|record| record.text.clone())
            .collect()
    }

    /// Whether an alternate-buffer switch occurred within the window.
    ///
    /// Compares the checkpoint at the window start against the current state,
    /// because comparing row indices across a buffer switch is meaningless.
    pub fn alt_screen_switched_since(&self, cutoff: Instant, now_alt: bool) -> bool {
        match self.checkpoint_at_or_before(cutoff) {
            Some(checkpoint) => checkpoint.alt_screen != now_alt,
            None => self
                .oldest_checkpoint()
                .is_some_and(|checkpoint| checkpoint.alt_screen != now_alt),
        }
    }

    pub fn checkpoint_count(&self) -> usize {
        self.checkpoints.len()
    }
}

#[cfg(test)]
mod tests {
    use super::{History, HistoryLimits};
    use std::time::{Duration, Instant};

    fn limits() -> HistoryLimits {
        HistoryLimits {
            checkpoint_interval: Duration::from_millis(100),
            checkpoint_capacity: 4,
            dirty_capacity: 4,
            output_capacity: 4,
            output_bytes: 64,
            scrollback_capacity: 4,
        }
    }

    #[test]
    fn dirty_rows_are_unioned_deduped_and_sorted() {
        let mut h = History::new(limits());
        let t0 = Instant::now();
        h.record_dirty(t0, vec![3, 1]);
        h.record_dirty(t0 + Duration::from_millis(10), vec![1, 5]);
        assert_eq!(h.dirty_rows_since(t0), vec![1, 3, 5]);
    }

    #[test]
    fn dirty_rows_before_the_cutoff_are_excluded() {
        let mut h = History::new(limits());
        let t0 = Instant::now();
        h.record_dirty(t0, vec![9]);
        h.record_dirty(t0 + Duration::from_millis(50), vec![2]);
        assert_eq!(h.dirty_rows_since(t0 + Duration::from_millis(25)), vec![2]);
    }

    #[test]
    fn empty_dirty_records_are_not_stored() {
        let mut h = History::new(limits());
        h.record_dirty(Instant::now(), vec![]);
        assert!(
            h.dirty_rows_since(Instant::now() - Duration::from_secs(1))
                .is_empty()
        );
        assert!(!h.has_evicted());
    }

    #[test]
    fn rings_evict_and_report_it() {
        let mut h = History::new(limits());
        let t0 = Instant::now();
        for i in 0..10 {
            h.record_dirty(t0 + Duration::from_millis(i), vec![i as usize]);
        }
        assert!(h.has_evicted());
        // Only the last `dirty_capacity` records survive.
        assert_eq!(h.dirty_rows_since(t0), vec![6, 7, 8, 9]);
    }

    #[test]
    fn output_ring_is_bounded_by_bytes_as_well_as_count() {
        let mut h = History::new(limits());
        let t0 = Instant::now();
        // One record well over the 64-byte budget forces eviction immediately.
        h.record_output(t0, "x".repeat(100));
        h.record_output(t0 + Duration::from_millis(1), "tail".into());
        assert!(h.has_evicted());
        assert_eq!(h.output_since(t0), "tail");
    }

    #[test]
    fn output_is_concatenated_in_order() {
        let mut h = History::new(limits());
        let t0 = Instant::now();
        h.record_output(t0, "a".into());
        h.record_output(t0 + Duration::from_millis(1), "b".into());
        assert_eq!(h.output_since(t0), "ab");
    }

    #[test]
    fn checkpoints_respect_the_interval() {
        let mut h = History::new(limits());
        let t0 = Instant::now();
        h.maybe_checkpoint(t0, false, || vec!["a".into()], false);
        // Too soon; skipped.
        h.maybe_checkpoint(
            t0 + Duration::from_millis(10),
            false,
            || vec!["b".into()],
            false,
        );
        assert_eq!(h.checkpoint_count(), 1);
        // Interval elapsed; taken.
        h.maybe_checkpoint(
            t0 + Duration::from_millis(150),
            false,
            || vec!["c".into()],
            false,
        );
        assert_eq!(h.checkpoint_count(), 2);
    }

    #[test]
    fn forced_checkpoint_bypasses_the_interval() {
        let mut h = History::new(limits());
        let t0 = Instant::now();
        h.maybe_checkpoint(t0, false, || vec!["a".into()], false);
        h.maybe_checkpoint(
            t0 + Duration::from_millis(1),
            true,
            || vec!["b".into()],
            true,
        );
        assert_eq!(h.checkpoint_count(), 2);
    }

    #[test]
    fn checkpoint_lookup_finds_the_latest_at_or_before_the_cutoff() {
        let mut h = History::new(limits());
        let t0 = Instant::now();
        h.maybe_checkpoint(t0, true, || vec!["first".into()], false);
        h.maybe_checkpoint(
            t0 + Duration::from_millis(200),
            true,
            || vec!["second".into()],
            false,
        );
        h.maybe_checkpoint(
            t0 + Duration::from_millis(400),
            true,
            || vec!["third".into()],
            false,
        );

        let found = h.checkpoint_at_or_before(t0 + Duration::from_millis(250));
        assert_eq!(
            found.map(|c| c.view.clone()),
            Some(vec!["second".to_string()])
        );
    }

    #[test]
    fn checkpoint_lookup_returns_none_when_window_predates_history() {
        let mut h = History::new(limits());
        let t0 = Instant::now();
        h.maybe_checkpoint(t0, true, || vec!["only".into()], false);
        assert!(
            h.checkpoint_at_or_before(t0 - Duration::from_secs(5))
                .is_none()
        );
    }

    #[test]
    fn alt_screen_switch_is_detected_against_the_window_start() {
        let mut h = History::new(limits());
        let t0 = Instant::now();
        h.maybe_checkpoint(t0, true, Vec::new, false);
        assert!(h.alt_screen_switched_since(t0, true));
        assert!(!h.alt_screen_switched_since(t0, false));
    }

    #[test]
    fn scrollback_eviction_is_recorded_and_bounded() {
        let mut h = History::new(limits());
        let t0 = Instant::now();
        h.record_scrollback(t0, (0..10).map(|i| format!("line{i}")));
        let kept = h.scrollback_since(t0);
        assert_eq!(kept.len(), 4);
        assert_eq!(kept[0], "line6");
        assert!(h.has_evicted());
    }
}
