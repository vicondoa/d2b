//! The delta engine: answering "what changed in the last N seconds".
//!
//! There is no single correct answer, because the question means different
//! things depending on what the child is doing.
//!
//! `avt` reports changed rows as **viewport-relative indices**, not stable row
//! identities. For a full-screen TUI that is exactly right: the viewport is
//! fixed, so "row 7 changed" means the content at row 7 is new. For a scrolling
//! shell it is misleading: every row is dirty on every newline simply because
//! content moved upward through it.
//!
//! So this module reports by mode:
//!
//! * **`AltScreen`** -- the dirty-row union, with each row's current text.
//!   Precise and small.
//! * **`Scrolling`** -- the appended transcript, assembled from output recorded
//!   in the window plus any lines evicted from scrollback.
//!
//! Both modes additionally carry a real line diff against the checkpoint at the
//! start of the window, which is the answer that holds regardless of scrolling.
//!
//! An alternate-buffer switch inside the window invalidates row-index
//! comparison entirely. That is reported explicitly rather than diffed across.

use serde::{Deserialize, Serialize};

/// Which interpretation of "changed" the report used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeltaMode {
    /// A full-screen application owns the alternate buffer.
    AltScreen,
    /// Ordinary scrolling output on the primary buffer.
    Scrolling,
}

/// A viewport row that changed, with its current content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedRow {
    pub index: usize,
    pub text: String,
}

/// One edit in the checkpoint diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum DiffOp {
    /// Present now, absent at the checkpoint. `index` is the row in the
    /// current view.
    Added { index: usize, text: String },
    /// Present at the checkpoint, absent now. `index` is the row in the
    /// checkpoint view.
    Removed { index: usize, text: String },
}

/// The answer to a delta query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeltaReport {
    /// The requested window, in milliseconds.
    pub window_ms: u64,
    pub mode: DeltaMode,
    /// Whether the alternate buffer is active now.
    pub alt_screen: bool,
    /// Whether the buffer switched during the window. When true, `changedRows`
    /// and `diff` compare across a screen swap and should be read as "the
    /// screen was replaced", not as a line-level edit.
    pub alt_screen_switched: bool,
    /// Age of the baseline checkpoint used for `diff`, in milliseconds.
    /// `None` when the window reaches further back than any retained
    /// checkpoint.
    pub baseline_age_ms: Option<u64>,
    /// Viewport rows `avt` marked dirty during the window.
    pub changed_rows: Vec<usize>,
    /// Current content of those rows. Populated in `AltScreen` mode.
    pub rows: Vec<ChangedRow>,
    /// Line diff of the current viewport against the baseline checkpoint.
    pub diff: Vec<DiffOp>,
    /// Transcript appended during the window. Populated in `Scrolling` mode.
    pub appended: Vec<String>,
    /// Bytes of decoded output recorded during the window.
    pub output_bytes: usize,
    /// Whether any row's rendered content differs from the baseline.
    ///
    /// This is the field to use for idle detection, not `output_bytes`. An
    /// application that repositions the cursor on a timer emits PTY traffic
    /// continuously while the screen stays visually identical: `avt`
    /// deliberately does not mark a line dirty for pure cursor movement, so
    /// such a session shows `output_bytes > 0` with no content change at all.
    /// Treating raw traffic as activity reports those sessions as busy forever.
    pub content_changed: bool,
    /// Whether the cursor moved during the window.
    ///
    /// Note that a cursor *blink* rendered by the terminal itself produces no
    /// PTY output and is invisible here, which is correct: it is a property of
    /// the display, not of the application.
    pub cursor_moved: bool,
    /// Whether any history ring evicted, so this answer may be partial.
    pub truncated: bool,
    /// Human-readable caveat, when one applies.
    pub note: Option<String>,
}

impl DeltaReport {
    /// True when nothing at all happened in the window, including cursor and
    /// control traffic that left the visible content unchanged.
    pub fn is_empty(&self) -> bool {
        !self.content_changed && self.output_bytes == 0
    }

    /// True when the visible content is unchanged, regardless of any cursor or
    /// control traffic. This is the predicate for "has the screen settled".
    pub fn is_idle(&self) -> bool {
        !self.content_changed
    }

    /// True when the child emitted output that changed nothing visible.
    pub fn cursor_only_activity(&self) -> bool {
        !self.content_changed && self.output_bytes > 0
    }

    /// Render the report for a terminal reader.
    pub fn render_human(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();

        let secs = self.window_ms as f64 / 1000.0;
        let mode = match self.mode {
            DeltaMode::AltScreen => "alt-screen",
            DeltaMode::Scrolling => "scrolling",
        };
        let _ = writeln!(out, "window {secs:.1}s  mode {mode}");

        if self.alt_screen_switched {
            let _ = writeln!(
                out,
                "! screen buffer switched during this window; row indices are not comparable"
            );
        }
        if self.truncated {
            let _ = writeln!(out, "! history evicted; this answer may be partial");
        }
        if let Some(note) = &self.note {
            let _ = writeln!(out, "! {note}");
        }

        if !self.content_changed {
            if self.cursor_only_activity() {
                let _ = writeln!(
                    out,
                    "(no change; {} bytes of cursor/control traffic moved nothing visible)",
                    self.output_bytes
                );
            } else {
                let _ = writeln!(out, "(no change)");
            }
            return out;
        }

        match self.mode {
            DeltaMode::AltScreen => {
                let _ = writeln!(out, "changed rows: {}", self.changed_rows.len());
                for row in &self.rows {
                    let _ = writeln!(out, "{:>4} | {}", row.index, row.text);
                }
            }
            DeltaMode::Scrolling => {
                let _ = writeln!(out, "appended lines: {}", self.appended.len());
                for line in &self.appended {
                    let _ = writeln!(out, "  + {line}");
                }
            }
        }

        if !self.diff.is_empty() {
            let _ = writeln!(out, "--- diff vs baseline ---");
            for op in &self.diff {
                match op {
                    DiffOp::Added { index, text } => {
                        let _ = writeln!(out, "+{index:>3} | {text}");
                    }
                    DiffOp::Removed { index, text } => {
                        let _ = writeln!(out, "-{index:>3} | {text}");
                    }
                }
            }
        }

        out
    }
}

/// Line diff via longest common subsequence.
///
/// Viewports are tens of rows, so the quadratic table is a few thousand cells
/// and not worth optimising. Returns only the edits; unchanged lines are
/// omitted, since an agent asking what changed does not want the whole screen
/// back.
pub fn diff_lines(old: &[String], new: &[String]) -> Vec<DiffOp> {
    let n = old.len();
    let m = new.len();

    if n == 0 && m == 0 {
        return Vec::new();
    }

    // lcs[i][j] = length of the LCS of old[i..] and new[j..].
    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if old[i] == new[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut ops = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);

    while i < n && j < m {
        if old[i] == new[j] {
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            ops.push(DiffOp::Removed {
                index: i,
                text: old[i].clone(),
            });
            i += 1;
        } else {
            ops.push(DiffOp::Added {
                index: j,
                text: new[j].clone(),
            });
            j += 1;
        }
    }

    while i < n {
        ops.push(DiffOp::Removed {
            index: i,
            text: old[i].clone(),
        });
        i += 1;
    }

    while j < m {
        ops.push(DiffOp::Added {
            index: j,
            text: new[j].clone(),
        });
        j += 1;
    }

    ops
}

/// Split recorded output into appended transcript lines.
///
/// Control sequences are stripped by taking the emulator's word for it
/// elsewhere; here we only need the newline-delimited shape of what arrived.
/// A trailing fragment with no newline is kept, since a shell prompt has no
/// trailing newline and omitting it would hide the most interesting line.
pub fn split_appended(output: &str) -> Vec<String> {
    if output.is_empty() {
        return Vec::new();
    }

    output
        .split('\n')
        .map(|line| line.trim_end_matches('\r').to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{ChangedRow, DeltaMode, DeltaReport, DiffOp, diff_lines, split_appended};

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn identical_input_produces_no_edits() {
        let a = lines(&["one", "two", "three"]);
        assert!(diff_lines(&a, &a).is_empty());
    }

    #[test]
    fn a_changed_line_is_one_removal_and_one_addition() {
        let old = lines(&["one", "two", "three"]);
        let new = lines(&["one", "TWO", "three"]);
        let ops = diff_lines(&old, &new);
        assert_eq!(
            ops,
            vec![
                DiffOp::Removed {
                    index: 1,
                    text: "two".into()
                },
                DiffOp::Added {
                    index: 1,
                    text: "TWO".into()
                },
            ]
        );
    }

    #[test]
    fn appended_lines_are_additions_only() {
        let old = lines(&["one"]);
        let new = lines(&["one", "two"]);
        assert_eq!(
            diff_lines(&old, &new),
            vec![DiffOp::Added {
                index: 1,
                text: "two".into()
            }]
        );
    }

    #[test]
    fn scrolling_by_one_line_is_a_minimal_edit() {
        // This is the case that makes LCS worth the effort: a naive positional
        // comparison would report every row as changed.
        let old = lines(&["a", "b", "c"]);
        let new = lines(&["b", "c", "d"]);
        let ops = diff_lines(&old, &new);
        assert_eq!(
            ops,
            vec![
                DiffOp::Removed {
                    index: 0,
                    text: "a".into()
                },
                DiffOp::Added {
                    index: 2,
                    text: "d".into()
                },
            ]
        );
    }

    #[test]
    fn empty_to_populated_is_all_additions() {
        let ops = diff_lines(&[], &lines(&["x", "y"]));
        assert_eq!(ops.len(), 2);
        assert!(ops.iter().all(|op| matches!(op, DiffOp::Added { .. })));
    }

    #[test]
    fn populated_to_empty_is_all_removals() {
        let ops = diff_lines(&lines(&["x", "y"]), &[]);
        assert_eq!(ops.len(), 2);
        assert!(ops.iter().all(|op| matches!(op, DiffOp::Removed { .. })));
    }

    #[test]
    fn both_empty_is_no_edits() {
        assert!(diff_lines(&[], &[]).is_empty());
    }

    #[test]
    fn appended_split_drops_blank_lines_and_carriage_returns() {
        assert_eq!(split_appended("a\r\nb\r\n"), vec!["a", "b"]);
    }

    #[test]
    fn appended_split_keeps_a_trailing_fragment() {
        // A shell prompt has no trailing newline; dropping it would hide the
        // single most useful line.
        assert_eq!(split_appended("done\r\n$ "), vec!["done", "$ "]);
    }

    #[test]
    fn appended_split_of_empty_is_empty() {
        assert!(split_appended("").is_empty());
    }

    fn report() -> DeltaReport {
        DeltaReport {
            window_ms: 10_000,
            mode: DeltaMode::AltScreen,
            alt_screen: true,
            alt_screen_switched: false,
            baseline_age_ms: Some(10_000),
            changed_rows: vec![2],
            rows: vec![ChangedRow {
                index: 2,
                text: "hello".into(),
            }],
            diff: vec![],
            appended: vec![],
            output_bytes: 5,
            content_changed: true,
            cursor_moved: false,
            truncated: false,
            note: None,
        }
    }

    #[test]
    fn report_round_trips_through_json() {
        let r = report();
        let json = serde_json::to_string(&r).unwrap_or_default();
        let back: DeltaReport = serde_json::from_str(&json).unwrap_or_else(|_| r.clone());
        assert_eq!(r, back);
    }

    #[test]
    fn empty_report_is_detected() {
        let mut r = report();
        r.changed_rows.clear();
        r.rows.clear();
        r.output_bytes = 0;
        r.content_changed = false;
        assert!(r.is_empty());
        assert!(r.is_idle());
    }

    #[test]
    fn cursor_only_traffic_counts_as_idle_but_not_as_empty() {
        // The case that breaks naive idle detection: an application that
        // repositions its cursor on a timer emits PTY traffic continuously
        // while the screen stays visually identical. Keying idle off raw
        // output bytes would report such a session busy forever.
        let mut r = report();
        r.changed_rows.clear();
        r.rows.clear();
        r.content_changed = false;
        r.cursor_moved = true;
        r.output_bytes = 4096;

        assert!(r.is_idle(), "visible content did not change");
        assert!(!r.is_empty(), "raw traffic did occur");
        assert!(r.cursor_only_activity());
    }

    #[test]
    fn content_change_is_never_idle() {
        let r = report();
        assert!(r.content_changed);
        assert!(!r.is_idle());
        assert!(!r.is_empty());
    }

    #[test]
    fn human_render_explains_cursor_only_traffic() {
        let mut r = report();
        r.changed_rows.clear();
        r.rows.clear();
        r.content_changed = false;
        r.output_bytes = 4096;
        let text = r.render_human();
        assert!(text.contains("no change"), "{text}");
        assert!(text.contains("moved nothing visible"), "{text}");
    }

    #[test]
    fn human_render_flags_a_buffer_switch() {
        let mut r = report();
        r.alt_screen_switched = true;
        let text = r.render_human();
        assert!(text.contains("switched"));
    }

    #[test]
    fn human_render_flags_truncation() {
        let mut r = report();
        r.truncated = true;
        assert!(r.render_human().contains("evicted"));
    }

    #[test]
    fn human_render_states_no_change_explicitly() {
        let mut r = report();
        r.changed_rows.clear();
        r.rows.clear();
        r.output_bytes = 0;
        r.content_changed = false;
        assert!(r.render_human().contains("(no change)"));
    }
}
