//! Terminal private-mode tracking.
//!
//! `avt` tracks the primary and alternate screen buffers internally but exposes
//! no accessor: `Vt`'s `terminal` field is private and there is no
//! `is_alternate()`. We need that bit for two reasons -- the agent wants to know
//! whether a full-screen TUI is on screen, and the delta engine must refuse to
//! diff row indices across a buffer switch.
//!
//! Rather than fork `avt`, we scan the output stream for the DEC private mode
//! set/reset sequences ourselves. The same scanner also picks up bracketed
//! paste (mode 2004), which `avt` does not model at all and which we need in
//! order to encode pasted text correctly.
//!
//! An upstream patch exposing `Terminal::active_buffer_type()` would let this
//! module drop the alternate-screen half; the bracketed-paste half would remain
//! regardless.

/// DEC private modes we care about.
///
/// 47, 1047 and 1049 all select the alternate screen buffer; they differ only
/// in whether the cursor is saved and whether the buffer is cleared, which is
/// `avt`'s problem rather than ours.
const ALT_SCREEN_MODES: [u16; 3] = [47, 1047, 1049];
const BRACKETED_PASTE_MODE: u16 = 2004;

/// Tracked terminal modes, updated by scanning the child's output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modes {
    pub alt_screen: bool,
    pub bracketed_paste: bool,
}

/// Incremental scanner for `CSI ? <params> h|l`.
///
/// Written as an explicit state machine so it survives a sequence split across
/// PTY reads, which is the same hazard `Utf8Decoder` handles for text.
#[derive(Debug, Default)]
pub struct ModeScanner {
    state: State,
    params: Vec<u16>,
    current: Option<u16>,
    modes: Modes,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum State {
    /// Ordinary text.
    #[default]
    Ground,
    /// Saw ESC.
    Escape,
    /// Saw `CSI`, awaiting the `?` private marker.
    Csi,
    /// Inside a private-mode parameter list.
    PrivateParams,
}

impl ModeScanner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn modes(&self) -> Modes {
        self.modes
    }

    /// Feed decoded text, updating tracked mode state.
    ///
    /// Takes `&str` rather than bytes because it sits downstream of
    /// [`crate::utf8::Utf8Decoder`], so it never sees a split character.
    pub fn feed(&mut self, text: &str) {
        for ch in text.chars() {
            self.step(ch);
        }
    }

    fn step(&mut self, ch: char) {
        match self.state {
            State::Ground => {
                if ch == '\u{1b}' {
                    self.state = State::Escape;
                } else if ch == '\u{9b}' {
                    // 8-bit CSI.
                    self.begin_csi();
                }
            }

            State::Escape => {
                if ch == '[' {
                    self.begin_csi();
                } else if ch == '\u{1b}' {
                    // Stay in Escape; a second ESC restarts the sequence.
                } else {
                    self.state = State::Ground;
                }
            }

            State::Csi => {
                if ch == '?' {
                    self.state = State::PrivateParams;
                    self.params.clear();
                    self.current = None;
                } else {
                    // Not a private-mode sequence; ignore the rest of it.
                    self.state = State::Ground;
                }
            }

            State::PrivateParams => match ch {
                '0'..='9' => {
                    let digit = u32::from(ch) - u32::from('0');
                    let acc = self.current.unwrap_or(0);
                    // Saturate rather than wrap on absurd input.
                    self.current = Some(acc.saturating_mul(10).saturating_add(digit as u16));
                }
                ';' => {
                    self.params.push(self.current.take().unwrap_or(0));
                }
                'h' | 'l' => {
                    if let Some(value) = self.current.take() {
                        self.params.push(value);
                    }
                    let enable = ch == 'h';
                    let params = std::mem::take(&mut self.params);
                    for param in params {
                        self.apply(param, enable);
                    }
                    self.state = State::Ground;
                }
                _ => {
                    // Some other private sequence (e.g. `CSI ? 6 n`). Drop it.
                    self.params.clear();
                    self.current = None;
                    self.state = State::Ground;
                }
            },
        }
    }

    fn begin_csi(&mut self) {
        self.state = State::Csi;
        self.params.clear();
        self.current = None;
    }

    fn apply(&mut self, param: u16, enable: bool) {
        if ALT_SCREEN_MODES.contains(&param) {
            self.modes.alt_screen = enable;
        } else if param == BRACKETED_PASTE_MODE {
            self.modes.bracketed_paste = enable;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ModeScanner;

    fn scan(chunks: &[&str]) -> super::Modes {
        let mut s = ModeScanner::new();
        for c in chunks {
            s.feed(c);
        }
        s.modes()
    }

    #[test]
    fn starts_on_primary_buffer() {
        let m = scan(&[""]);
        assert!(!m.alt_screen);
        assert!(!m.bracketed_paste);
    }

    #[test]
    fn mode_1049_enters_and_leaves_alt_screen() {
        assert!(scan(&["\x1b[?1049h"]).alt_screen);
        assert!(!scan(&["\x1b[?1049h", "\x1b[?1049l"]).alt_screen);
    }

    #[test]
    fn legacy_alt_screen_modes_are_recognised() {
        assert!(scan(&["\x1b[?47h"]).alt_screen);
        assert!(scan(&["\x1b[?1047h"]).alt_screen);
    }

    #[test]
    fn bracketed_paste_is_tracked_independently() {
        let m = scan(&["\x1b[?2004h"]);
        assert!(m.bracketed_paste);
        assert!(!m.alt_screen);
    }

    #[test]
    fn multi_parameter_sequence_sets_every_mode() {
        let m = scan(&["\x1b[?1049;2004h"]);
        assert!(m.alt_screen);
        assert!(m.bracketed_paste);
    }

    #[test]
    fn sequence_split_across_feeds_is_still_recognised() {
        // This is the case a naive per-chunk regex would miss.
        assert!(scan(&["\x1b[?10", "49h"]).alt_screen);
        assert!(scan(&["\x1b", "[?1049h"]).alt_screen);
        assert!(scan(&["\x1b[", "?", "1", "0", "4", "9", "h"]).alt_screen);
    }

    #[test]
    fn eight_bit_csi_is_recognised() {
        assert!(scan(&["\u{9b}?1049h"]).alt_screen);
    }

    #[test]
    fn unrelated_sequences_do_not_disturb_state() {
        let m = scan(&["\x1b[?1049h", "\x1b[31m", "text", "\x1b[?25l", "\x1b[2J"]);
        assert!(m.alt_screen);
    }

    #[test]
    fn non_private_csi_is_ignored() {
        // `CSI 4 h` is ANSI insert mode, not a DEC private mode.
        assert!(!scan(&["\x1b[4h"]).alt_screen);
    }

    #[test]
    fn cursor_position_report_does_not_clear_modes() {
        let m = scan(&["\x1b[?1049h", "\x1b[?6n"]);
        assert!(m.alt_screen);
    }
}
