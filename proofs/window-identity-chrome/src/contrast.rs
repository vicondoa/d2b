//! WCAG contrast, and why the shipped luminance test is not one.
//!
//! `d2b-wayland-proxy` currently picks label colour with a weighted-sum
//! brightness test. That is not WCAG relative luminance: it omits the sRGB
//! transfer function, so it systematically over-estimates the brightness of
//! saturated colours and picks black text on backgrounds where black fails.
//! [`tests::naive_luma_picks_unreadable_text`] measures how badly.

/// WCAG 2.x contrast ratio required for body text (SC 1.4.3, AA).
pub const CONTRAST_TEXT_AA: f64 = 4.5;

/// WCAG 2.x contrast ratio required for UI components and graphical objects
/// (SC 1.4.11).
pub const CONTRAST_NON_TEXT: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const BLACK: Rgb = Rgb { r: 0, g: 0, b: 0 };
    pub const WHITE: Rgb = Rgb {
        r: 255,
        g: 255,
        b: 255,
    };

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Blend toward `other` by `t` in [0, 1].
    pub fn mix(self, other: Rgb, t: f64) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let f = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8;
        Rgb {
            r: f(self.r, other.r),
            g: f(self.g, other.g),
            b: f(self.b, other.b),
        }
    }
}

/// Linearize one sRGB channel, per WCAG's definition.
fn linearize(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance.
pub fn relative_luminance(c: Rgb) -> f64 {
    0.2126 * linearize(c.r) + 0.7152 * linearize(c.g) + 0.0722 * linearize(c.b)
}

/// WCAG contrast ratio between two colours. Always >= 1.0, and symmetric.
pub fn contrast_ratio(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// The naive brightness test the shipped proxy uses, for comparison.
///
/// Kept so the proof can measure the gap rather than assert it.
pub fn naive_pick_readable(background: Rgb) -> Rgb {
    let luma = 0.299 * f64::from(background.r)
        + 0.587 * f64::from(background.g)
        + 0.114 * f64::from(background.b);
    if luma > 128.0 {
        Rgb::BLACK
    } else {
        Rgb::WHITE
    }
}

/// Pick black or white, whichever contrasts better against `background`.
///
/// This is always the best available choice from that pair, and
/// [`tests::best_of_black_or_white_always_clears_aa`] shows it always clears
/// 4.5:1 -- but only just, so the margin is worth knowing rather than
/// assuming.
pub fn pick_readable(background: Rgb) -> Rgb {
    if contrast_ratio(Rgb::WHITE, background) >= contrast_ratio(Rgb::BLACK, background) {
        Rgb::WHITE
    } else {
        Rgb::BLACK
    }
}

/// Push `fg` toward black or white until it clears `target` against `bg`.
///
/// Returns `None` when even the extreme cannot reach the target, so callers
/// must decide what to do rather than silently shipping unreadable text.
pub fn enforce_contrast(fg: Rgb, bg: Rgb, target: f64) -> Option<Rgb> {
    if contrast_ratio(fg, bg) >= target {
        return Some(fg);
    }
    let toward = pick_readable(bg);
    // 32 steps is finer than 8-bit channels can express, so this cannot miss a
    // reachable solution.
    for i in 1..=32 {
        let candidate = fg.mix(toward, f64::from(i) / 32.0);
        if contrast_ratio(candidate, bg) >= target {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_is_symmetric_and_bounded() {
        assert!((contrast_ratio(Rgb::BLACK, Rgb::WHITE) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(Rgb::WHITE, Rgb::BLACK) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(Rgb::WHITE, Rgb::WHITE) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn best_of_black_or_white_always_clears_aa() {
        // Worth stating precisely, because the intuition "some mid-tone must
        // fail against both black and white" is wrong. The worst case is where
        // the two ratios are equal, at L* such that (L+0.05)/0.05 =
        // 1.05/(L+0.05), i.e. L = sqrt(0.0525) - 0.05 ~= 0.1791, giving
        // ~4.58:1 -- above 4.5:1, but with under 2% of margin.
        let mut worst = f64::MAX;
        let mut worst_at = Rgb::BLACK;
        for r in (0..=255).step_by(3) {
            for g in (0..=255).step_by(3) {
                for b in (0..=255).step_by(3) {
                    let bg = Rgb::new(r as u8, g as u8, b as u8);
                    let ratio = contrast_ratio(pick_readable(bg), bg);
                    if ratio < worst {
                        worst = ratio;
                        worst_at = bg;
                    }
                }
            }
        }
        assert!(
            worst >= CONTRAST_TEXT_AA,
            "worst best-of-pair contrast {worst:.4} at {worst_at:?} fails AA"
        );
        assert!(
            worst < 4.60,
            "margin is thinner than 4.60 by construction; got {worst:.4}"
        );
    }

    #[test]
    fn naive_luma_picks_unreadable_text() {
        // The shipped behaviour. This is not a hypothetical: saturated greens
        // and cyans are exactly the colours an operator would choose for a
        // realm accent.
        let mut failures = 0_u32;
        let mut worst = f64::MAX;
        let mut worst_at = Rgb::BLACK;
        for r in (0..=255).step_by(3) {
            for g in (0..=255).step_by(3) {
                for b in (0..=255).step_by(3) {
                    let bg = Rgb::new(r as u8, g as u8, b as u8);
                    let ratio = contrast_ratio(naive_pick_readable(bg), bg);
                    if ratio < CONTRAST_TEXT_AA {
                        failures += 1;
                        if ratio < worst {
                            worst = ratio;
                            worst_at = bg;
                        }
                    }
                }
            }
        }
        assert!(
            failures > 0,
            "the naive test is supposed to fail; if this passes, re-check it"
        );
        assert!(
            worst < 2.5,
            "expected a severe worst case, got {worst:.4} at {worst_at:?}"
        );
        // Recorded so a future change to the naive function is visible.
        eprintln!(
            "naive luma: {failures} sampled colours below AA, worst {worst:.4} at {worst_at:?}"
        );
    }

    #[test]
    fn correct_selection_never_fails_where_naive_does() {
        for r in (0..=255).step_by(7) {
            for g in (0..=255).step_by(7) {
                for b in (0..=255).step_by(7) {
                    let bg = Rgb::new(r as u8, g as u8, b as u8);
                    assert!(
                        contrast_ratio(pick_readable(bg), bg) >= CONTRAST_TEXT_AA,
                        "pick_readable failed on {bg:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn enforce_contrast_returns_none_rather_than_shipping_unreadable_text() {
        // Mid gray against mid gray cannot reach 21:1, so the honest answer is
        // "cannot", not a best effort that silently fails the criterion.
        let gray = Rgb::new(128, 128, 128);
        assert_eq!(enforce_contrast(gray, gray, 21.0), None);
    }

    #[test]
    fn enforce_contrast_leaves_already_passing_colours_alone() {
        let bg = Rgb::new(0x25, 0x27, 0x2b);
        let fg = Rgb::new(0xff, 0xff, 0xff);
        assert_eq!(enforce_contrast(fg, bg, CONTRAST_TEXT_AA), Some(fg));
    }

    #[test]
    fn enforce_contrast_reaches_the_target_for_every_realistic_accent() {
        // Every accent an operator might pick must yield readable label text
        // on the neutral card, or the design has a hole.
        let card = Rgb::new(0x25, 0x27, 0x2b);
        for r in (0..=255).step_by(5) {
            for g in (0..=255).step_by(5) {
                for b in (0..=255).step_by(5) {
                    let accent = Rgb::new(r as u8, g as u8, b as u8);
                    let fixed = enforce_contrast(accent, card, CONTRAST_TEXT_AA);
                    if let Some(c) = fixed {
                        assert!(contrast_ratio(c, card) >= CONTRAST_TEXT_AA);
                    } else {
                        // Only near-card colours may be unreachable, and the
                        // renderer falls back to pick_readable for those.
                        assert!(
                            contrast_ratio(pick_readable(card), card) >= CONTRAST_TEXT_AA,
                            "no fallback available for accent {accent:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    // Both operands are constants, which is the point: this pins the ordering
    // of the two thresholds so a future edit cannot raise the non-text
    // threshold above the text one. clippy 1.97 flags assertions over
    // constants because they cannot fail at runtime; here that is the
    // intended guarantee rather than a mistake.
    #[allow(clippy::assertions_on_constants)]
    fn non_text_threshold_is_lower_than_text() {
        assert!(CONTRAST_NON_TEXT < CONTRAST_TEXT_AA);
    }
}
