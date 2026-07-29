//! Identity text: sanitization and ellipsization.
//!
//! The label is a security surface. It is the thing an operator reads to
//! decide which realm they are about to type a password into, so a workload
//! name must not be able to lie about itself.
//!
//! The interesting case is bidirectional control characters. `char::is_control`
//! covers Unicode category Cc only; the overrides that reverse rendered text
//! (U+202E RIGHT-TO-LEFT OVERRIDE and friends) are category Cf and pass
//! straight through a naive filter. `work\u{202E}lamron` renders as
//! `worknormal`, which is exactly the attack.

/// Longest label rendered before ellipsization, in characters.
pub const MAX_LABEL_CHARS: usize = 32;

/// Characters that reorder or hide rendered text.
///
/// Listed explicitly rather than by category, because the property that
/// matters is "changes what the reader sees", not any single Unicode class.
const DECEPTIVE: &[char] = &[
    '\u{200E}', // LEFT-TO-RIGHT MARK
    '\u{200F}', // RIGHT-TO-LEFT MARK
    '\u{202A}', // LEFT-TO-RIGHT EMBEDDING
    '\u{202B}', // RIGHT-TO-LEFT EMBEDDING
    '\u{202C}', // POP DIRECTIONAL FORMATTING
    '\u{202D}', // LEFT-TO-RIGHT OVERRIDE
    '\u{202E}', // RIGHT-TO-LEFT OVERRIDE
    '\u{2066}', // LEFT-TO-RIGHT ISOLATE
    '\u{2067}', // RIGHT-TO-LEFT ISOLATE
    '\u{2068}', // FIRST STRONG ISOLATE
    '\u{2069}', // POP DIRECTIONAL ISOLATE
    '\u{200B}', // ZERO WIDTH SPACE
    '\u{200C}', // ZERO WIDTH NON-JOINER
    '\u{200D}', // ZERO WIDTH JOINER
    '\u{FEFF}', // ZERO WIDTH NO-BREAK SPACE
    '\u{00AD}', // SOFT HYPHEN
];

/// A label that is safe to render.
///
/// Constructing one is the only way to get text into the chrome, so the
/// sanitization cannot be skipped at a call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeLabel(String);

impl SafeLabel {
    /// Sanitize `raw`, returning `None` if nothing renderable survives.
    ///
    /// Returning `None` rather than an empty label matters: an empty label
    /// would draw an unlabelled tab, which is the failure this surface exists
    /// to prevent. The caller must fail closed instead.
    pub fn new(raw: &str) -> Option<Self> {
        let cleaned: String = raw
            .chars()
            .filter(|c| !c.is_control() && !DECEPTIVE.contains(c))
            // Collapse anything that is not a printable glyph into a space, so
            // exotic separators cannot be used to fake structure.
            .map(|c| if c.is_whitespace() { ' ' } else { c })
            .collect();

        let collapsed = collapse_spaces(&cleaned);
        if collapsed.is_empty() {
            return None;
        }
        Some(SafeLabel(collapsed))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The label as drawn, ellipsized to `max_chars`.
    pub fn display(&self, max_chars: usize) -> String {
        ellipsize(&self.0, max_chars)
    }
}

fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for c in s.chars() {
        if c == ' ' {
            if !last_space && !out.is_empty() {
                out.push(c);
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Truncate to `max_chars` including the ellipsis.
///
/// Truncation keeps the *start* of the name, because that is where realm and
/// workload identity live; truncating the start would let two different
/// workloads render identically.
pub fn ellipsize(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    if max_chars == 1 {
        return "…".to_string();
    }
    let keep = max_chars - 1;
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_names_survive_unchanged() {
        for name in ["work", "personal", "corp-workstation.work", "VM 1"] {
            assert_eq!(SafeLabel::new(name).unwrap().as_str(), name);
        }
    }

    #[test]
    fn bidi_overrides_are_stripped() {
        // The attack: a workload named so that it renders as another realm.
        let deceptive = "work\u{202E}lamron";
        let safe = SafeLabel::new(deceptive).unwrap();
        assert_eq!(safe.as_str(), "worklamron");
        assert!(!safe.as_str().contains('\u{202E}'));
    }

    #[test]
    fn every_deceptive_character_is_removed() {
        for c in DECEPTIVE {
            let raw = format!("a{c}b");
            let safe = SafeLabel::new(&raw).unwrap();
            assert_eq!(safe.as_str(), "ab", "{c:?} survived sanitization");
        }
    }

    #[test]
    fn is_control_alone_would_not_have_caught_these() {
        // Documents *why* the explicit list exists: the dangerous ones are
        // category Cf, and `char::is_control` covers only Cc.
        let cf = ['\u{202E}', '\u{202D}', '\u{2067}', '\u{200B}'];
        for c in cf {
            assert!(
                !c.is_control(),
                "{c:?} is Cc after all; the explicit list may be redundant"
            );
            assert!(DECEPTIVE.contains(&c), "{c:?} must be in the list");
        }
    }

    #[test]
    fn control_characters_are_stripped() {
        let safe = SafeLabel::new("work\n\r\tdev").unwrap();
        assert_eq!(safe.as_str(), "workdev");
    }

    #[test]
    fn whitespace_is_collapsed_so_structure_cannot_be_faked() {
        // "work            personal" must not be able to look like two labels.
        let safe = SafeLabel::new("work            personal").unwrap();
        assert_eq!(safe.as_str(), "work personal");
    }

    #[test]
    fn leading_and_trailing_whitespace_is_removed() {
        assert_eq!(SafeLabel::new("   work   ").unwrap().as_str(), "work");
    }

    #[test]
    fn an_empty_result_is_none_rather_than_an_empty_label() {
        // Fail closed: an empty label would render an unlabelled tab.
        assert_eq!(SafeLabel::new(""), None);
        assert_eq!(SafeLabel::new("   "), None);
        assert_eq!(SafeLabel::new("\u{202E}\u{200B}"), None);
    }

    #[test]
    fn ellipsize_keeps_the_start_of_the_name() {
        // Two workloads that differ only in their suffix must not render the
        // same; keeping the start is what preserves realm and workload.
        let long = "corp-workstation.work";
        let short = ellipsize(long, 10);
        assert_eq!(short.chars().count(), 10);
        assert!(short.starts_with("corp-work"));
        assert!(short.ends_with('…'));
    }

    #[test]
    fn ellipsize_leaves_short_names_alone() {
        assert_eq!(ellipsize("work", MAX_LABEL_CHARS), "work");
    }

    #[test]
    fn ellipsize_degrades_sanely_at_tiny_budgets() {
        assert_eq!(ellipsize("work", 0), "");
        assert_eq!(ellipsize("work", 1), "…");
        assert_eq!(ellipsize("work", 2), "w…");
    }

    #[test]
    fn ellipsize_is_char_safe_for_multibyte_names() {
        let s = "wörk-Ω-виртуалка";
        let out = ellipsize(s, 6);
        assert_eq!(out.chars().count(), 6);
        // Must not panic and must remain valid UTF-8, which it is by type.
        assert!(out.ends_with('…'));
    }

    #[test]
    fn display_applies_the_budget() {
        let safe = SafeLabel::new("corp-workstation.work").unwrap();
        assert!(safe.display(8).chars().count() <= 8);
        assert_eq!(safe.display(MAX_LABEL_CHARS), "corp-workstation.work");
    }
}
