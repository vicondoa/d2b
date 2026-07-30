//! Session state: the emulator, the mode scanner, and the history rings.
//!
//! Owns everything derived from child output. The pump feeds it; the socket
//! server queries it.

use std::time::{Duration, Instant};

use crate::delta::{ChangedRow, DeltaMode, DeltaReport, DiffOp, diff_lines};
use crate::history::{History, HistoryLimits};
use crate::modes::ModeScanner;
use crate::screen::{ScreenSnapshot, read_cursor, render_primary_text, render_view};
use crate::tty::TtySize;
use crate::utf8::Utf8Decoder;

/// Default scrollback retained by the emulator, in lines.
pub const DEFAULT_SCROLLBACK: usize = 5000;

/// Everything the agent can observe.
pub struct Session {
    vt: avt::Vt,
    scanner: ModeScanner,
    decoder: Utf8Decoder,
    history: History,
    size: TtySize,
    child_pid: i32,
    started: Instant,
    exit_status: Option<i32>,
}

impl Session {
    pub fn new(size: TtySize, scrollback: usize, child_pid: i32) -> Self {
        let vt = avt::Vt::builder()
            .size(size.cols as usize, size.rows as usize)
            .scrollback_limit(scrollback)
            .build();

        let mut session = Self {
            vt,
            scanner: ModeScanner::new(),
            decoder: Utf8Decoder::new(),
            history: History::new(HistoryLimits::default()),
            size,
            child_pid,
            started: Instant::now(),
            exit_status: None,
        };

        // Seed a baseline checkpoint so a delta query in the first half-second
        // has something to compare against.
        let now = session.started;
        let view = render_view(&session.vt);
        session.history.maybe_checkpoint(now, true, || view, false);

        session
    }

    pub fn size(&self) -> TtySize {
        self.size
    }

    pub fn child_pid(&self) -> i32 {
        self.child_pid
    }

    pub fn uptime(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn exit_status(&self) -> Option<i32> {
        self.exit_status
    }

    pub fn set_exit_status(&mut self, status: i32) {
        self.exit_status = Some(status);
    }

    /// Whether the child has DECCKM set, which decides whether arrow keys are
    /// encoded as `CSI A` or `SS3 A`.
    pub fn cursor_key_app_mode(&self) -> bool {
        self.vt.cursor_key_app_mode()
    }

    /// Whether the child has bracketed paste enabled.
    pub fn bracketed_paste(&self) -> bool {
        self.scanner.modes().bracketed_paste
    }

    /// Whether a full-screen application owns the alternate buffer.
    pub fn alt_screen(&self) -> bool {
        self.scanner.modes().alt_screen
    }

    /// A sequence that reconstructs the current screen on a blank terminal.
    ///
    /// This is `avt`'s `dump()`, and it is how a late joiner catches up exactly
    /// -- cursor, pen, margins, charset and alternate-buffer state included --
    /// in bounded size, rather than by replaying a raw byte log.
    pub fn dump(&self) -> String {
        self.vt.dump()
    }

    /// Feed raw child output into the emulator and the history rings.
    pub fn feed_output(&mut self, bytes: &[u8]) {
        let text = self.decoder.feed(bytes);
        if text.is_empty() {
            return;
        }
        self.ingest(&text);
    }

    /// Flush a truncated trailing character at end of stream.
    pub fn finish_output(&mut self) {
        let text = self.decoder.flush();
        if !text.is_empty() {
            self.ingest(&text);
        }
    }

    fn ingest(&mut self, text: &str) {
        let now = Instant::now();
        let was_alt = self.scanner.modes().alt_screen;

        self.scanner.feed(text);
        let is_alt = self.scanner.modes().alt_screen;

        // Destructure so the evicted scrollback iterator is drained before the
        // mutable borrow of `vt` ends.
        let (dirty, evicted) = {
            let changes = self.vt.feed_str(text);
            let dirty = changes.lines;
            let evicted: Vec<String> = changes.scrollback.map(|line| line.text()).collect();
            (dirty, evicted)
        };

        self.history.record_output(now, text.to_string());
        self.history.record_dirty(now, dirty);
        if !evicted.is_empty() {
            self.history.record_scrollback(now, evicted);
        }

        // A buffer switch invalidates row-index comparison, so force a fresh
        // baseline at exactly that instant.
        let switched = was_alt != is_alt;
        let view = render_view(&self.vt);
        self.history
            .maybe_checkpoint(now, switched, || view, is_alt);
    }

    /// Take a checkpoint if the interval has elapsed, without new output.
    ///
    /// The pump calls this on a timer. Without it, checkpoints only exist at
    /// instants when output happened, so once a screen settles the newest
    /// baseline ages indefinitely and a later query diffs against a stale
    /// screen. An idle terminal still needs a current baseline.
    pub fn checkpoint_now(&mut self) {
        let now = Instant::now();
        let is_alt = self.scanner.modes().alt_screen;
        let view = render_view(&self.vt);
        self.history.maybe_checkpoint(now, false, || view, is_alt);
    }

    /// Resize the emulator. The caller is responsible for the PTY ioctl.
    pub fn resize(&mut self, size: TtySize) {
        if size == self.size {
            return;
        }

        self.size = size;
        let now = Instant::now();

        // Collect inside the borrow scope; a reflow that evicts lines would
        // otherwise drop them silently.
        let (dirty, evicted) = {
            let changes = self.vt.resize(size.cols as usize, size.rows as usize);
            let dirty = changes.lines;
            let evicted: Vec<String> = changes.scrollback.map(|line| line.text()).collect();
            (dirty, evicted)
        };

        if !evicted.is_empty() {
            self.history.record_scrollback(now, evicted);
        }
        self.history.record_dirty(now, dirty);

        // A resize reflows every row, so the previous baseline is not
        // comparable. Force a new one.
        let is_alt = self.scanner.modes().alt_screen;
        let view = render_view(&self.vt);
        self.history.maybe_checkpoint(now, true, || view, is_alt);
    }

    /// A point-in-time view of the terminal.
    pub fn snapshot(&self) -> ScreenSnapshot {
        let modes = self.scanner.modes();
        ScreenSnapshot {
            cols: self.size.cols as usize,
            rows: self.size.rows as usize,
            cursor: read_cursor(&self.vt),
            alt_screen: modes.alt_screen,
            bracketed_paste: modes.bracketed_paste,
            view: render_view(&self.vt),
            primary_text: render_primary_text(&self.vt),
        }
    }

    /// Answer a delta query over the trailing `window`.
    pub fn delta(&self, window: Duration) -> DeltaReport {
        let now = Instant::now();
        let cutoff = now.checked_sub(window).unwrap_or(self.started);

        let alt_screen = self.scanner.modes().alt_screen;
        let mode = if alt_screen {
            DeltaMode::AltScreen
        } else {
            DeltaMode::Scrolling
        };

        let changed_rows = self.history.dirty_rows_since(cutoff);
        let output = self.history.output_since(cutoff);

        // Exact short circuit. Every screen change originates in child output,
        // and a resize records dirty rows, so if neither happened in the window
        // then nothing changed. Answering from the checkpoint diff instead
        // would report spurious changes whenever the newest baseline predates
        // the window, which is the common case for a settled screen.
        if output.is_empty() && changed_rows.is_empty() {
            return DeltaReport {
                window_ms: window.as_millis() as u64,
                mode,
                alt_screen,
                alt_screen_switched: false,
                baseline_age_ms: self
                    .history
                    .checkpoint_at_or_before(cutoff)
                    .map(|c| now.duration_since(c.at).as_millis() as u64),
                changed_rows: Vec::new(),
                rows: Vec::new(),
                diff: Vec::new(),
                appended: Vec::new(),
                output_bytes: 0,
                truncated: self.history.has_evicted(),
                note: None,
            };
        }

        let current = render_view(&self.vt);

        // Pick the baseline. Falling back to the oldest retained checkpoint is
        // better than refusing to answer, but the caller must be told that the
        // window was clamped.
        let mut note: Option<String> = None;
        let baseline = match self.history.checkpoint_at_or_before(cutoff) {
            Some(checkpoint) => Some(checkpoint),
            None => {
                let oldest = self.history.oldest_checkpoint();
                if oldest.is_some() {
                    note = Some(
                        "requested window predates retained history; \
                         compared against the oldest checkpoint instead"
                            .to_string(),
                    );
                }
                oldest
            }
        };

        let baseline_age_ms = baseline.map(|c| now.duration_since(c.at).as_millis() as u64);
        let diff = match baseline {
            Some(checkpoint) => diff_lines(&checkpoint.view, &current),
            None => Vec::new(),
        };

        // Only populate the row texts in alt-screen mode. In scrolling mode a
        // dirty row index means "content moved through here", so echoing those
        // rows back would be actively misleading.
        let rows = if alt_screen {
            changed_rows
                .iter()
                .filter_map(|&index| {
                    current.get(index).map(|text| ChangedRow {
                        index,
                        text: text.clone(),
                    })
                })
                .collect()
        } else {
            Vec::new()
        };

        // The appended transcript comes from the diff rather than the raw
        // output ring, so it carries rendered text instead of escape sequences.
        let appended = if alt_screen {
            Vec::new()
        } else {
            let mut appended: Vec<String> = self
                .history
                .scrollback_since(cutoff)
                .into_iter()
                .filter(|line| !line.trim().is_empty())
                .collect();
            appended.extend(diff.iter().filter_map(|op| match op {
                DiffOp::Added { text, .. } if !text.trim().is_empty() => Some(text.clone()),
                _ => None,
            }));
            appended
        };

        DeltaReport {
            window_ms: window.as_millis() as u64,
            mode,
            alt_screen,
            alt_screen_switched: self.history.alt_screen_switched_since(cutoff, alt_screen),
            baseline_age_ms,
            changed_rows,
            rows,
            diff,
            appended,
            output_bytes: output.len(),
            truncated: self.history.has_evicted(),
            note,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_SCROLLBACK, Session};
    use crate::delta::DeltaMode;
    use crate::tty::TtySize;
    use std::time::Duration;

    fn session() -> Session {
        Session::new(TtySize::new(20, 4), DEFAULT_SCROLLBACK, 1234)
    }

    #[test]
    fn output_reaches_the_screen() {
        let mut s = session();
        s.feed_output(b"hello");
        let snap = s.snapshot();
        assert_eq!(snap.view[0], "hello");
        assert_eq!(snap.cursor.col, 5);
        assert_eq!(snap.cursor.row, 0);
    }

    #[test]
    fn multibyte_output_split_across_feeds_is_intact() {
        let mut s = session();
        s.feed_output(&[0xE6, 0x97]);
        s.feed_output(&[0xA5]);
        assert!(s.snapshot().view[0].contains('\u{65e5}'));
    }

    #[test]
    fn alt_screen_is_tracked_from_the_output_stream() {
        let mut s = session();
        assert!(!s.alt_screen());
        s.feed_output(b"\x1b[?1049h");
        assert!(s.alt_screen());
        s.feed_output(b"\x1b[?1049l");
        assert!(!s.alt_screen());
    }

    #[test]
    fn bracketed_paste_is_tracked_from_the_output_stream() {
        let mut s = session();
        assert!(!s.bracketed_paste());
        s.feed_output(b"\x1b[?2004h");
        assert!(s.bracketed_paste());
    }

    #[test]
    fn application_cursor_mode_is_read_from_the_emulator() {
        let mut s = session();
        assert!(!s.cursor_key_app_mode());
        s.feed_output(b"\x1b[?1h");
        assert!(s.cursor_key_app_mode());
    }

    #[test]
    fn delta_reports_scrolling_mode_on_the_primary_buffer() {
        let mut s = session();
        s.feed_output(b"one\r\n");
        let report = s.delta(Duration::from_secs(10));
        assert_eq!(report.mode, DeltaMode::Scrolling);
        assert!(!report.alt_screen);
    }

    #[test]
    fn delta_reports_alt_screen_mode_and_row_texts() {
        let mut s = session();
        s.feed_output(b"\x1b[?1049h");
        s.feed_output(b"\x1b[2;1Hstatus line");
        let report = s.delta(Duration::from_secs(10));
        assert_eq!(report.mode, DeltaMode::AltScreen);
        assert!(report.rows.iter().any(|r| r.text.contains("status line")));
    }

    #[test]
    fn delta_flags_a_buffer_switch_inside_the_window() {
        let mut s = session();
        s.feed_output(b"shell output\r\n");
        s.feed_output(b"\x1b[?1049h");
        let report = s.delta(Duration::from_secs(10));
        assert!(report.alt_screen_switched);
    }

    #[test]
    fn delta_over_a_quiet_window_is_empty() {
        let s = session();
        let report = s.delta(Duration::from_secs(10));
        assert!(report.is_empty(), "unexpected change: {report:?}");
    }

    #[test]
    fn delta_is_empty_once_output_falls_outside_the_window() {
        // Regression. Checkpoints are only taken when output arrives, so after
        // a screen settles the newest baseline can predate the query window.
        // Diffing against it reported the whole screen as freshly changed
        // forever. The delta must be driven by whether anything happened in
        // the window, not by how old the baseline happens to be.
        let mut s = session();
        s.feed_output(b"some output\r\n");

        std::thread::sleep(Duration::from_millis(60));

        let report = s.delta(Duration::from_millis(20));
        assert!(
            report.is_empty(),
            "settled screen reported as changed: {report:?}"
        );
        assert!(report.diff.is_empty());
        assert!(report.appended.is_empty());
        assert_eq!(report.output_bytes, 0);
    }

    #[test]
    fn delta_still_reports_output_inside_the_window() {
        // The other side of the short circuit: it must not swallow real change.
        let mut s = session();
        s.feed_output(b"real output\r\n");
        let report = s.delta(Duration::from_secs(10));
        assert!(!report.is_empty());
        assert!(report.appended.iter().any(|l| l.contains("real output")));
    }

    #[test]
    fn idle_checkpoint_refreshes_the_baseline() {
        let mut s = session();
        s.feed_output(b"content\r\n");
        std::thread::sleep(Duration::from_millis(60));
        // The pump calls this on a timer; it must not panic and must leave the
        // session queryable.
        s.checkpoint_now();
        assert!(s.delta(Duration::from_millis(20)).is_empty());
    }

    #[test]
    fn resize_alone_registers_as_a_change() {
        // A resize produces dirty rows without any child output, so the short
        // circuit must consider dirty rows as well as output bytes.
        let mut s = session();
        s.feed_output(b"content\r\n");
        std::thread::sleep(Duration::from_millis(60));
        assert!(s.delta(Duration::from_millis(20)).is_empty());

        s.resize(TtySize::new(40, 8));
        let report = s.delta(Duration::from_millis(20));
        assert!(!report.changed_rows.is_empty(), "{report:?}");
    }

    #[test]
    fn delta_appended_carries_rendered_text_not_escape_sequences() {
        let mut s = session();
        s.feed_output(b"\x1b[32mgreen\x1b[0m\r\n");
        let report = s.delta(Duration::from_secs(10));
        assert!(report.appended.iter().any(|l| l.contains("green")));
        assert!(
            report.appended.iter().all(|l| !l.contains('\x1b')),
            "escape sequences leaked into the transcript: {:?}",
            report.appended
        );
    }

    #[test]
    fn delta_counts_output_bytes() {
        let mut s = session();
        s.feed_output(b"abcdef");
        assert_eq!(s.delta(Duration::from_secs(10)).output_bytes, 6);
    }

    #[test]
    fn resize_updates_the_reported_size() {
        let mut s = session();
        s.resize(TtySize::new(40, 10));
        let snap = s.snapshot();
        assert_eq!(snap.cols, 40);
        assert_eq!(snap.rows, 10);
    }

    #[test]
    fn dump_reconstructs_the_screen_in_a_fresh_emulator() {
        // This is the property that makes late-join catch-up exact.
        let mut s = session();
        s.feed_output(b"line one\r\nline two");

        let mut replay = avt::Vt::builder().size(20, 4).build();
        replay.feed_str(&s.dump());

        let original: Vec<String> = s.snapshot().view;
        let restored: Vec<String> = replay
            .view()
            .map(|l| l.text().trim_end().to_string())
            .collect();

        assert_eq!(original, restored);
    }

    #[test]
    fn dump_reconstructs_alt_screen_state() {
        let mut s = session();
        s.feed_output(b"\x1b[?1049h");
        s.feed_output(b"\x1b[1;1HTUI");

        let mut replay = avt::Vt::builder().size(20, 4).build();
        replay.feed_str(&s.dump());

        let restored: Vec<String> = replay
            .view()
            .map(|l| l.text().trim_end().to_string())
            .collect();
        assert_eq!(restored, s.snapshot().view);
    }

    #[test]
    fn primary_text_survives_an_alt_screen_takeover() {
        // avt::Vt::text() always reads the primary buffer. That is a trap if
        // you assume it follows view(), but it means the shell transcript is
        // still visible while a TUI owns the screen.
        let mut s = session();
        s.feed_output(b"shell line\r\n");
        s.feed_output(b"\x1b[?1049h");
        s.feed_output(b"\x1b[1;1HTUI");

        let snap = s.snapshot();
        assert!(snap.view.iter().any(|l| l.contains("TUI")));
        assert!(snap.primary_text.iter().any(|l| l.contains("shell line")));
    }
}
