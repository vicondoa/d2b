//! Key-name grammar and escape-sequence encoding.
//!
//! Derived from `ht` (headless terminal), `src/api/stdio.rs::parse_key`,
//! version 0.4.0, commit ed569e91a7a8930faea7d1364b2175747f8a54d2, Apache-2.0.
//!
//! `ht` is by Andy Konwinski, @andyk (<https://github.com/andyk/ht>); that file
//! is largely the work of Marcin Kulik, @ku1ik (<https://github.com/ku1ik>),
//! with a contribution from @MatrixManAtYrService. **This file has been
//! modified** -- see README.md part 6 for the full attribution and the list of
//! changes.
//!
//! The key names follow tmux's convention, which is what `ht` adopted:
//!
//! ```text
//! Enter Space Escape Tab Backspace Insert Delete
//! Left Right Up Down Home End PageUp PageDown
//! F1 .. F12
//! ^x        control, ASCII letters only
//! C-x       control
//! S-x       shift, special keys only
//! A-x       alt
//! ```
//!
//! Modifiers combine in any order, so `C-S-Left`, `S-C-Left` and `S-A-C-Left`
//! are all accepted.
//!
//! # Modification from ht
//!
//! `ht` enumerates every modifier permutation as an explicit match arm, which
//! is why its table only supports combined modifiers on arrow keys. This
//! version parses modifiers into a bitset and computes the xterm modifier
//! parameter arithmetically, so every modifier combination works on every key.
//! The generated sequences are byte-identical to ht's for every input ht
//! accepts.

use std::fmt;

/// Bracketed paste start, `CSI 200 ~`.
const PASTE_START: &str = "\x1b[200~";
/// Bracketed paste end, `CSI 201 ~`.
const PASTE_END: &str = "\x1b[201~";

/// An encoded key, resolved to bytes once the cursor-key mode is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySeq {
    /// A fixed sequence, independent of terminal modes.
    Standard(String),
    /// `(normal, application)`. Which one applies depends on DECCKM, which we
    /// read from `avt`'s `cursor_key_app_mode()`.
    Cursor(String, String),
}

impl KeySeq {
    /// Resolve to the bytes to write to the PTY.
    pub fn resolve(&self, app_mode: bool) -> &str {
        match self {
            KeySeq::Standard(s) => s,
            KeySeq::Cursor(normal, app) => {
                if app_mode {
                    app
                } else {
                    normal
                }
            }
        }
    }
}

/// A key name that carried a modifier prefix we could not apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyParseError {
    pub key: String,
    pub reason: &'static str,
}

impl fmt::Display for KeyParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot encode key {:?}: {}", self.key, self.reason)
    }
}

impl std::error::Error for KeyParseError {}

/// Parsed modifier set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Mods {
    shift: bool,
    alt: bool,
    ctrl: bool,
}

impl Mods {
    fn any(self) -> bool {
        self.shift || self.alt || self.ctrl
    }

    /// The xterm modifier parameter: `1 + shift(1) + alt(2) + ctrl(4)`.
    fn xterm_param(self) -> u8 {
        1 + u8::from(self.shift) + 2 * u8::from(self.alt) + 4 * u8::from(self.ctrl)
    }
}

/// Shape of a key's escape sequence, which determines how modifiers attach.
enum Base {
    /// `CSI <final>` normally, `SS3 <final>` in application cursor mode.
    /// Modified always uses `CSI 1;<m> <final>`. Arrows, Home, End.
    CursorLike(char),
    /// `SS3 <final>` unmodified, `CSI 1;<m> <final>` modified. F1-F4.
    Ss3(char),
    /// `CSI <n> ~` unmodified, `CSI <n>;<m> ~` modified.
    /// PageUp/PageDown/Insert/Delete/F5-F12.
    Tilde(u16),
    /// A literal sequence that takes no modifier encoding.
    Literal(&'static str),
}

/// Look up the base sequence for an unmodified key name.
fn base_for(name: &str) -> Option<Base> {
    let base = match name {
        // Cursor-mode-sensitive keys.
        "Left" => Base::CursorLike('D'),
        "Right" => Base::CursorLike('C'),
        "Up" => Base::CursorLike('A'),
        "Down" => Base::CursorLike('B'),
        "Home" => Base::CursorLike('H'),
        "End" => Base::CursorLike('F'),

        // Function keys.
        "F1" => Base::Ss3('P'),
        "F2" => Base::Ss3('Q'),
        "F3" => Base::Ss3('R'),
        "F4" => Base::Ss3('S'),
        "F5" => Base::Tilde(15),
        "F6" => Base::Tilde(17),
        "F7" => Base::Tilde(18),
        "F8" => Base::Tilde(19),
        "F9" => Base::Tilde(20),
        "F10" => Base::Tilde(21),
        "F11" => Base::Tilde(23),
        "F12" => Base::Tilde(24),

        // Navigation and editing. Insert and Delete are additions over ht.
        "PageUp" => Base::Tilde(5),
        "PageDown" => Base::Tilde(6),
        "Insert" => Base::Tilde(2),
        "Delete" => Base::Tilde(3),

        // Fixed literals.
        "Tab" => Base::Literal("\x09"),
        "Enter" => Base::Literal("\x0d"),
        "Backspace" => Base::Literal("\x7f"),
        "Escape" => Base::Literal("\x1b"),
        "Space" => Base::Literal(" "),
        // Shift-Tab. An addition over ht.
        "BackTab" => Base::Literal("\x1b[Z"),

        _ => return None,
    };

    Some(base)
}

/// Encode a base sequence with a modifier set applied.
///
/// The four `Base` shapes differ in where the modifier parameter goes, which is
/// why they are modelled separately rather than as flat strings:
///
/// ```text
/// CursorLike('A')  unmodified  ESC [ A      (or ESC O A in application mode)
///                  modified    ESC [ 1;<m> A
/// Ss3('P')         unmodified  ESC O P
///                  modified    ESC [ 1;<m> P
/// Tilde(15)        unmodified  ESC [ 15 ~
///                  modified    ESC [ 15;<m> ~
/// Literal("\t")    no modifier encoding at all
/// ```
fn encode(base: Base, mods: Mods) -> KeySeq {
    match base {
        Base::CursorLike(final_ch) => {
            if mods.any() {
                // Once a modifier is present the CSI form is always used, even
                // in application cursor mode. This matches xterm and is what
                // `ht` did.
                KeySeq::Standard(format!("\x1b[1;{}{}", mods.xterm_param(), final_ch))
            } else {
                // Deferred: the caller resolves this once DECCKM is known.
                KeySeq::Cursor(format!("\x1b[{final_ch}"), format!("\x1bO{final_ch}"))
            }
        }

        Base::Ss3(final_ch) => {
            if mods.any() {
                KeySeq::Standard(format!("\x1b[1;{}{}", mods.xterm_param(), final_ch))
            } else {
                KeySeq::Standard(format!("\x1bO{final_ch}"))
            }
        }

        Base::Tilde(n) => {
            if mods.any() {
                KeySeq::Standard(format!("\x1b[{};{}~", n, mods.xterm_param()))
            } else {
                KeySeq::Standard(format!("\x1b[{n}~"))
            }
        }

        Base::Literal(s) => {
            // Shift-Tab is the one literal with a conventional modified form.
            if s == "\x09"
                && mods
                    == (Mods {
                        shift: true,
                        ..Mods::default()
                    })
            {
                KeySeq::Standard("\x1b[Z".to_string())
            } else if mods.alt {
                // Alt-prefix any literal by preceding it with ESC.
                KeySeq::Standard(format!("\x1b{s}"))
            } else {
                KeySeq::Standard(s.to_string())
            }
        }
    }
}

/// Strip modifier prefixes, returning the modifier set and the bare key name.
///
/// Loops rather than matching a fixed order, which is what makes `C-S-Left`,
/// `S-C-Left` and `S-A-C-Left` all equivalent. `ht` enumerated every
/// permutation by hand and so only supported them on arrow keys.
fn split_mods(key: &str) -> (Mods, &str) {
    let mut mods = Mods::default();
    let mut rest = key;

    loop {
        if let Some(tail) = rest.strip_prefix("C-") {
            mods.ctrl = true;
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("S-") {
            mods.shift = true;
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("A-") {
            mods.alt = true;
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("M-") {
            // tmux spells alt as `M-` (meta) in some contexts.
            mods.alt = true;
            rest = tail;
        } else {
            break;
        }
    }

    (mods, rest)
}

/// Map a control-modified ASCII letter or symbol to its control code.
///
/// This preserves ht's exact mapping, including its choice to map both `C-^`
/// and `C-/` to 0x1E. (0x1F is the more common convention for `C-/`, but
/// deviating silently from the vendored table would be worse than inheriting
/// its choice.)
fn ctrl_code(name: &str) -> Option<&'static str> {
    let code = match name {
        "@" | "Space" => "\x00",
        "[" => "\x1b",
        "\\" => "\x1c",
        "]" => "\x1d",
        "^" | "/" => "\x1e",
        "-" | "_" => "\x1f",
        "?" => "\x7f",
        _ => return None,
    };

    Some(code)
}

/// Parse one key specification into an encodable sequence.
///
/// A name that matches nothing and carries no modifier is passed through as
/// literal text, which is ht's documented behaviour and is what makes
/// `keys hello Enter` work.
pub fn parse_key(key: &str) -> Result<KeySeq, KeyParseError> {
    // Caret control notation, ASCII letters and symbols only.
    if let Some(rest) = key.strip_prefix('^')
        && key.len() > 1
    {
        if let Some(code) = ctrl_code(rest) {
            return Ok(KeySeq::Standard(code.to_string()));
        }
        if let Some(code) = ctrl_letter(rest) {
            return Ok(KeySeq::Standard(code));
        }
        return Err(KeyParseError {
            key: key.to_string(),
            reason: "caret control notation accepts only ASCII letters and @[\\]^_-/?",
        });
    }

    let (mods, name) = split_mods(key);

    // An unmodified, unrecognised name is literal text.
    if !mods.any() {
        return match base_for(name) {
            Some(base) => Ok(encode(base, mods)),
            None => Ok(KeySeq::Standard(key.to_string())),
        };
    }

    // Control applied to a symbol or to Space, e.g. `C-[`, `C-Space`.
    //
    // This must be tried before `base_for`, because `Space` appears in both
    // tables: as the literal " " and as the control code NUL. Checking the
    // base table first would silently turn `C-Space` into a space.
    if mods.ctrl
        && !mods.shift
        && let Some(code) = ctrl_code(name)
    {
        let seq = if mods.alt {
            format!("\x1b{code}")
        } else {
            code.to_string()
        };
        return Ok(KeySeq::Standard(seq));
    }

    // A known special key with modifiers.
    if let Some(base) = base_for(name) {
        return Ok(encode(base, mods));
    }

    // Control applied to an ASCII letter, e.g. `C-c`.
    if mods.ctrl
        && let Some(code) = ctrl_letter(name)
    {
        let seq = if mods.alt {
            format!("\x1b{code}")
        } else {
            code
        };
        return Ok(KeySeq::Standard(seq));
    }

    // Alt applied to any single character, e.g. `A-x`.
    if mods.alt && !mods.ctrl && !mods.shift {
        let mut chars = name.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            return Ok(KeySeq::Standard(format!("\x1b{ch}")));
        }
    }

    // A modifier prefix was clearly intended but cannot be applied. ht would
    // silently send this as literal text, which types garbage into the child.
    Err(KeyParseError {
        key: key.to_string(),
        reason: "modifier prefix cannot be applied to this key name",
    })
}

/// Map a single ASCII letter to its control code.
fn ctrl_letter(name: &str) -> Option<String> {
    let mut chars = name.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }

    let code = match ch {
        'a'..='z' => (ch as u8) - 0x60,
        'A'..='Z' => (ch as u8) - 0x40,
        _ => return None,
    };

    Some((code as char).to_string())
}

/// Encode a list of key specifications into PTY bytes.
pub fn keys_to_bytes(keys: &[String], app_mode: bool) -> Result<Vec<u8>, KeyParseError> {
    let mut out = Vec::new();
    for key in keys {
        let seq = parse_key(key)?;
        out.extend_from_slice(seq.resolve(app_mode).as_bytes());
    }
    Ok(out)
}

/// Encode literal text for the PTY.
///
/// When the child has enabled bracketed paste (DEC mode 2004) the text is
/// wrapped in the paste markers, so an editor treats it as a paste rather than
/// as typing. Without this, pasting into vim triggers auto-indent cascades.
/// `ht` does not implement this at all.
pub fn text_to_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    if !bracketed {
        return text.as_bytes().to_vec();
    }

    // Strip any embedded end-marker so the payload cannot terminate its own
    // paste and inject the remainder as keystrokes.
    let sanitised = text.replace(PASTE_END, "");

    let mut out = Vec::with_capacity(PASTE_START.len() + sanitised.len() + PASTE_END.len());
    out.extend_from_slice(PASTE_START.as_bytes());
    out.extend_from_slice(sanitised.as_bytes());
    out.extend_from_slice(PASTE_END.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::{keys_to_bytes, parse_key, text_to_bytes};

    fn seq(key: &str) -> String {
        match parse_key(key) {
            Ok(k) => k.resolve(false).to_string(),
            Err(e) => panic!("{e}"),
        }
    }

    fn app_seq(key: &str) -> String {
        match parse_key(key) {
            Ok(k) => k.resolve(true).to_string(),
            Err(e) => panic!("{e}"),
        }
    }

    // --- parity with ht's table -------------------------------------------

    #[test]
    fn literal_keys_match_ht() {
        assert_eq!(seq("Enter"), "\x0d");
        assert_eq!(seq("Tab"), "\x09");
        assert_eq!(seq("Backspace"), "\x7f");
        assert_eq!(seq("Escape"), "\x1b");
        assert_eq!(seq("Space"), " ");
    }

    #[test]
    fn arrows_respect_application_cursor_mode() {
        assert_eq!(seq("Up"), "\x1b[A");
        assert_eq!(app_seq("Up"), "\x1bOA");
        assert_eq!(seq("Down"), "\x1b[B");
        assert_eq!(app_seq("Down"), "\x1bOB");
        assert_eq!(seq("Right"), "\x1b[C");
        assert_eq!(app_seq("Right"), "\x1bOC");
        assert_eq!(seq("Left"), "\x1b[D");
        assert_eq!(app_seq("Left"), "\x1bOD");
    }

    #[test]
    fn home_and_end_respect_application_cursor_mode() {
        assert_eq!(seq("Home"), "\x1b[H");
        assert_eq!(app_seq("Home"), "\x1bOH");
        assert_eq!(seq("End"), "\x1b[F");
        assert_eq!(app_seq("End"), "\x1bOF");
    }

    #[test]
    fn modified_arrows_match_ht_exactly() {
        assert_eq!(seq("C-Left"), "\x1b[1;5D");
        assert_eq!(seq("C-Right"), "\x1b[1;5C");
        assert_eq!(seq("C-Up"), "\x1b[1;5A");
        assert_eq!(seq("C-Down"), "\x1b[1;5B");
        assert_eq!(seq("S-Left"), "\x1b[1;2D");
        assert_eq!(seq("A-Left"), "\x1b[1;3D");
        assert_eq!(seq("C-S-Left"), "\x1b[1;6D");
        assert_eq!(seq("A-S-Up"), "\x1b[1;4A");
        assert_eq!(seq("C-A-Down"), "\x1b[1;7B");
        assert_eq!(seq("C-A-S-Right"), "\x1b[1;8C");
    }

    #[test]
    fn modifiers_are_order_insensitive() {
        // ht enumerated these permutations by hand; we derive them.
        assert_eq!(seq("C-S-Left"), seq("S-C-Left"));
        assert_eq!(seq("C-A-S-Up"), seq("S-A-C-Up"));
        assert_eq!(seq("A-C-Right"), seq("C-A-Right"));
    }

    #[test]
    fn modified_arrows_ignore_application_mode() {
        // ht always emits the CSI form once a modifier is present.
        assert_eq!(app_seq("C-Left"), "\x1b[1;5D");
    }

    #[test]
    fn function_keys_match_ht() {
        assert_eq!(seq("F1"), "\x1bOP");
        assert_eq!(seq("F4"), "\x1bOS");
        assert_eq!(seq("F5"), "\x1b[15~");
        assert_eq!(seq("F12"), "\x1b[24~");
        assert_eq!(seq("C-F1"), "\x1b[1;5P");
        assert_eq!(seq("C-F5"), "\x1b[15;5~");
        assert_eq!(seq("S-F1"), "\x1b[1;2P");
        assert_eq!(seq("A-F12"), "\x1b[24;3~");
    }

    #[test]
    fn paging_keys_match_ht() {
        assert_eq!(seq("PageUp"), "\x1b[5~");
        assert_eq!(seq("PageDown"), "\x1b[6~");
        assert_eq!(seq("C-PageUp"), "\x1b[5;5~");
        assert_eq!(seq("S-PageDown"), "\x1b[6;2~");
    }

    #[test]
    fn control_letters_match_ht() {
        assert_eq!(seq("C-c"), "\x03");
        assert_eq!(seq("C-C"), "\x03");
        assert_eq!(seq("^c"), "\x03");
        assert_eq!(seq("^C"), "\x03");
        assert_eq!(seq("C-a"), "\x01");
        assert_eq!(seq("C-x"), "\x18");
        assert_eq!(seq("^x"), "\x18");
    }

    #[test]
    fn control_symbols_match_ht() {
        assert_eq!(seq("C-@"), "\x00");
        assert_eq!(seq("C-Space"), "\x00");
        assert_eq!(seq("^@"), "\x00");
        assert_eq!(seq("C-["), "\x1b");
        assert_eq!(seq("C-\\"), "\x1c");
        assert_eq!(seq("C-]"), "\x1d");
        assert_eq!(seq("C-^"), "\x1e");
        assert_eq!(seq("C-_"), "\x1f");
    }

    #[test]
    fn alt_prefixes_any_character() {
        assert_eq!(seq("A-x"), "\x1bx");
        assert_eq!(seq("A-1"), "\x1b1");
    }

    #[test]
    fn unknown_unmodified_name_is_literal_text() {
        // This is what makes `keys nano Enter` work.
        assert_eq!(seq("nano"), "nano");
        assert_eq!(seq("hello world"), "hello world");
    }

    // --- additions over ht -------------------------------------------------

    #[test]
    fn insert_and_delete_are_supported() {
        assert_eq!(seq("Insert"), "\x1b[2~");
        assert_eq!(seq("Delete"), "\x1b[3~");
        assert_eq!(seq("C-Delete"), "\x1b[3;5~");
    }

    #[test]
    fn shift_tab_produces_backtab() {
        assert_eq!(seq("S-Tab"), "\x1b[Z");
        assert_eq!(seq("BackTab"), "\x1b[Z");
    }

    #[test]
    fn meta_is_an_alias_for_alt() {
        assert_eq!(seq("M-x"), seq("A-x"));
    }

    #[test]
    fn misapplied_modifier_is_an_error_not_garbage() {
        // ht sends "S-nano" to the child as literal text. We refuse.
        assert!(parse_key("S-nano").is_err());
        assert!(parse_key("C-nano").is_err());
    }

    // --- batching and paste ------------------------------------------------

    #[test]
    fn key_batch_concatenates_in_order() {
        let keys = vec!["ls".to_string(), "Enter".to_string()];
        assert_eq!(keys_to_bytes(&keys, false).ok(), Some(b"ls\x0d".to_vec()));
    }

    #[test]
    fn key_batch_propagates_error() {
        let keys = vec!["ok".to_string(), "S-nope".to_string()];
        assert!(keys_to_bytes(&keys, false).is_err());
    }

    #[test]
    fn plain_text_is_unwrapped_when_paste_mode_is_off() {
        assert_eq!(text_to_bytes("hi", false), b"hi".to_vec());
    }

    #[test]
    fn text_is_wrapped_when_paste_mode_is_on() {
        assert_eq!(text_to_bytes("hi", true), b"\x1b[200~hi\x1b[201~".to_vec());
    }

    #[test]
    fn embedded_end_marker_cannot_escape_the_paste() {
        let hostile = "a\x1b[201~rm -rf /\x0d";
        let out = text_to_bytes(hostile, true);
        let rendered = String::from_utf8_lossy(&out);
        assert_eq!(rendered.matches("\x1b[201~").count(), 1);
        assert!(rendered.ends_with("\x1b[201~"));
    }
}
