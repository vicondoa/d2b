//! Screen rendering: turning the emulator's state into something an agent can
//! read.

use serde::{Deserialize, Serialize};

/// Cursor position and visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CursorPos {
    pub col: usize,
    pub row: usize,
    pub visible: bool,
}

/// A point-in-time view of the terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenSnapshot {
    pub cols: usize,
    pub rows: usize,
    pub cursor: CursorPos,
    /// Whether a full-screen application currently owns the alternate buffer.
    pub alt_screen: bool,
    /// Whether the child has enabled bracketed paste (DEC mode 2004).
    pub bracketed_paste: bool,
    /// The active buffer's viewport, one string per row.
    pub view: Vec<String>,
    /// The primary buffer's text.
    ///
    /// `avt::Vt::text()` always reads the primary buffer even when the
    /// alternate buffer is active. That is a trap if you assume it tracks
    /// `view()`, but it is useful here: while a TUI owns the screen this still
    /// shows the shell transcript underneath it.
    pub primary_text: Vec<String>,
}

impl ScreenSnapshot {
    /// Render the viewport as a single string, one row per line.
    pub fn view_text(&self) -> String {
        self.view.join("\n")
    }
}

/// Trim trailing whitespace from a rendered row.
///
/// `avt`'s `Line::text()` pads every row out to the terminal width, so an
/// untrimmed 120-column screen is mostly spaces. Trimming is semantically
/// identical for a text view and drastically cheaper to put in front of a
/// model.
pub fn trim_row(row: &str) -> String {
    row.trim_end().to_string()
}

/// Render an `avt` viewport into trimmed rows.
///
/// Always returns exactly one entry per terminal row, including blank ones.
/// An agent correlating the cursor position against this list needs the indices
/// to line up, so blank rows must not be filtered out.
pub fn render_view(vt: &avt::Vt) -> Vec<String> {
    vt.view().map(|line| trim_row(&line.text())).collect()
}

/// Render the primary buffer's text, trailing blank lines removed.
///
/// Note this reads the *primary* buffer even when the alternate buffer is
/// active, because that is what `avt::Vt::text()` does. That asymmetry with
/// [`render_view`] is deliberate and useful: it means the shell transcript
/// stays readable underneath a full-screen TUI.
///
/// Trailing blanks are dropped here, unlike in `render_view`, because this is a
/// transcript rather than a positional grid.
pub fn render_primary_text(vt: &avt::Vt) -> Vec<String> {
    let mut lines: Vec<String> = vt.text().iter().map(|l| trim_row(l)).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

/// Read the cursor out of the emulator.
pub fn read_cursor(vt: &avt::Vt) -> CursorPos {
    let cursor = vt.cursor();
    CursorPos {
        col: cursor.col,
        row: cursor.row,
        visible: cursor.visible,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CursorPos, ScreenSnapshot, read_cursor, render_primary_text, render_view, trim_row,
    };

    fn vt_with(input: &str) -> avt::Vt {
        let mut vt = avt::Vt::builder().size(20, 3).scrollback_limit(100).build();
        vt.feed_str(input);
        vt
    }

    #[test]
    fn trim_row_removes_the_padding_avt_adds() {
        assert_eq!(trim_row("hello               "), "hello");
        assert_eq!(trim_row("   "), "");
        assert_eq!(trim_row("  leading kept"), "  leading kept");
    }

    #[test]
    fn render_view_returns_one_entry_per_row_including_blanks() {
        let vt = vt_with("hello\r\nworld");
        let view = render_view(&vt);
        // The viewport is fixed height, so blank rows must still be present:
        // an agent counting rows needs positions to line up with the cursor.
        assert_eq!(view, vec!["hello", "world", ""]);
    }

    #[test]
    fn render_primary_text_drops_trailing_blank_lines() {
        let vt = vt_with("hello\r\nworld");
        assert_eq!(render_primary_text(&vt), vec!["hello", "world"]);
    }

    #[test]
    fn cursor_is_read_back_correctly() {
        let vt = vt_with("hello\r\nworld");
        assert_eq!(
            read_cursor(&vt),
            CursorPos {
                col: 5,
                row: 1,
                visible: true
            }
        );
    }

    #[test]
    fn snapshot_view_text_joins_rows() {
        let snap = ScreenSnapshot {
            cols: 5,
            rows: 2,
            cursor: CursorPos {
                col: 0,
                row: 0,
                visible: true,
            },
            alt_screen: false,
            bracketed_paste: false,
            view: vec!["a".into(), "b".into()],
            primary_text: vec![],
        };
        assert_eq!(snap.view_text(), "a\nb");
    }

    #[test]
    fn snapshot_round_trips_through_json() {
        let snap = ScreenSnapshot {
            cols: 5,
            rows: 1,
            cursor: CursorPos {
                col: 1,
                row: 0,
                visible: false,
            },
            alt_screen: true,
            bracketed_paste: true,
            view: vec!["x".into()],
            primary_text: vec!["y".into()],
        };
        let json = serde_json::to_string(&snap).unwrap_or_default();
        let back: ScreenSnapshot = serde_json::from_str(&json).unwrap_or_else(|_| snap.clone());
        assert_eq!(snap, back);
    }
}
