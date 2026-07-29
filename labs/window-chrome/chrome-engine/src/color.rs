//! WCAG-correct colour handling for identity chrome.
//!
//! The shipping proxy picks label colour with `0.299R + 0.587G + 0.114B > 128000`,
//! which is a video-encoding luma approximation, not perceptual luminance. It
//! disagrees with WCAG on exactly the colours an identity palette is full of:
//! saturated mid-tones. Everything here uses the normative formula instead.

/// Straight (non-premultiplied) 8-bit colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    pub const TRANSPARENT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Parse `#rrggbb` or `#rrggbbaa`.
    pub fn parse_hex(input: &str) -> Result<Self, String> {
        let v = input
            .strip_prefix('#')
            .ok_or_else(|| format!("expected #rrggbb[aa], got `{input}`"))?;
        if !v.bytes().all(|b| b.is_ascii_hexdigit()) || (v.len() != 6 && v.len() != 8) {
            return Err(format!("expected #rrggbb[aa], got `{input}`"));
        }
        let byte = |i: usize| u8::from_str_radix(&v[i..i + 2], 16).unwrap_or(0);
        Ok(Self {
            r: byte(0),
            g: byte(2),
            b: byte(4),
            a: if v.len() == 8 { byte(6) } else { 255 },
        })
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// ARGB8888 little-endian bytes, the wl_shm format the proxy uses.
    ///
    /// Straight (non-premultiplied) alpha. Correct for PNG output; NOT correct
    /// for `wl_shm`, which expects premultiplied alpha — see
    /// [`Self::argb8888_premultiplied`].
    pub fn argb8888(self) -> [u8; 4] {
        [self.b, self.g, self.r, self.a]
    }

    /// ARGB8888 little-endian bytes with colour channels premultiplied by
    /// alpha, as `wl_shm`'s `Argb8888` format requires.
    ///
    /// Submitting straight-alpha values to a premultiplied format makes
    /// partially transparent pixels render far too bright, which shows up as a
    /// white fringe along every antialiased curve.
    pub fn argb8888_premultiplied(self) -> [u8; 4] {
        let mul = |c: u8| ((u16::from(c) * u16::from(self.a) + 127) / 255) as u8;
        [mul(self.b), mul(self.g), mul(self.r), self.a]
    }

    pub fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// Composite `self` over `under`, both straight alpha.
    pub fn over(self, under: Self) -> Self {
        if self.a == 255 {
            return self;
        }
        if self.a == 0 {
            return under;
        }
        let sa = f64::from(self.a) / 255.0;
        let ua = f64::from(under.a) / 255.0;
        let out_a = sa + ua * (1.0 - sa);
        if out_a <= f64::EPSILON {
            return Self::TRANSPARENT;
        }
        let mix = |s: u8, u: u8| {
            let v = (f64::from(s) * sa + f64::from(u) * ua * (1.0 - sa)) / out_a;
            v.round().clamp(0.0, 255.0) as u8
        };
        Self {
            r: mix(self.r, under.r),
            g: mix(self.g, under.g),
            b: mix(self.b, under.b),
            a: (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
        }
    }

    /// Blend toward `other` by `t` in [0,1]. Used for hover/press states.
    pub fn mix(self, other: Self, t: f64) -> Self {
        let t = t.clamp(0.0, 1.0);
        let lerp = |a: u8, b: u8| {
            (f64::from(a) + (f64::from(b) - f64::from(a)) * t)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        Self {
            r: lerp(self.r, other.r),
            g: lerp(self.g, other.g),
            b: lerp(self.b, other.b),
            a: lerp(self.a, other.a),
        }
    }

    /// ITU-R BT.601 luma, used ONLY to model a grayscale display.
    pub fn to_grayscale(self) -> Self {
        let y = (0.299 * f64::from(self.r) + 0.587 * f64::from(self.g) + 0.114 * f64::from(self.b))
            .round()
            .clamp(0.0, 255.0) as u8;
        Self {
            r: y,
            g: y,
            b: y,
            a: self.a,
        }
    }
}

/// WCAG 2.x relative luminance.
///
/// <https://www.w3.org/TR/WCAG22/#dfn-relative-luminance>
pub fn relative_luminance(c: Rgba) -> f64 {
    fn channel(v: u8) -> f64 {
        let s = f64::from(v) / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
}

/// WCAG contrast ratio, in [1.0, 21.0]. Order-independent.
pub fn contrast_ratio(a: Rgba, b: Rgba) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Contrast floors we hold chrome to.
pub const CONTRAST_TEXT_AA: f64 = 4.5; // WCAG 1.4.3, normal-size text
pub const CONTRAST_LARGE_TEXT_AA: f64 = 3.0; // WCAG 1.4.3, large text
pub const CONTRAST_NON_TEXT_AA: f64 = 3.0; // WCAG 1.4.11, UI components

/// Pick the readable foreground for `bg` using real luminance, not luma.
pub fn readable_on(bg: Rgba) -> Rgba {
    if contrast_ratio(Rgba::BLACK, bg) >= contrast_ratio(Rgba::WHITE, bg) {
        Rgba::BLACK
    } else {
        Rgba::WHITE
    }
}

/// Whether `fg` on `bg` clears a contrast floor.
pub fn meets_contrast(fg: Rgba, bg: Rgba, min: f64) -> bool {
    contrast_ratio(fg, bg) >= min - 1e-9
}

/// Nudge `fg` toward black or white until it clears `min` against `bg`.
///
/// Returns `None` when even pure black/white cannot reach the floor, which is
/// the caller's signal to change the design rather than ship failing contrast.
pub fn enforce_contrast(fg: Rgba, bg: Rgba, min: f64) -> Option<Rgba> {
    if meets_contrast(fg, bg, min) {
        return Some(fg);
    }
    let target = readable_on(bg);
    if !meets_contrast(target, bg, min) {
        return None;
    }
    // Binary search the smallest push toward `target` that clears the floor,
    // so we keep as much of the requested hue as the floor allows.
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if meets_contrast(fg.mix(target, mid), bg, min) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Some(fg.mix(target, hi))
}

/// Perceptual lightness difference in grayscale, for the monochrome-display
/// requirement: two identity colours must stay distinguishable without hue.
pub fn grayscale_delta(a: Rgba, b: Rgba) -> f64 {
    (relative_luminance(a.to_grayscale()) - relative_luminance(b.to_grayscale())).abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luminance_matches_wcag_reference_points() {
        assert!((relative_luminance(Rgba::BLACK) - 0.0).abs() < 1e-9);
        assert!((relative_luminance(Rgba::WHITE) - 1.0).abs() < 1e-9);
        // #808080 is the classic mid-gray reference: ~0.2159.
        let mid = relative_luminance(Rgba::rgb(0x80, 0x80, 0x80));
        assert!((mid - 0.2159).abs() < 1e-3, "got {mid}");
    }

    #[test]
    fn contrast_black_on_white_is_21() {
        let r = contrast_ratio(Rgba::BLACK, Rgba::WHITE);
        assert!((r - 21.0).abs() < 1e-6, "got {r}");
        // Order must not matter.
        assert!((contrast_ratio(Rgba::WHITE, Rgba::BLACK) - r).abs() < 1e-9);
    }

    #[test]
    fn identical_colours_have_ratio_one() {
        assert!((contrast_ratio(Rgba::rgb(1, 2, 3), Rgba::rgb(1, 2, 3)) - 1.0).abs() < 1e-9);
    }

    /// The regression that motivates this module: the shipping proxy's naive
    /// luma threshold disagrees with WCAG on saturated identity colours.
    #[test]
    fn naive_luma_disagrees_with_wcag_on_identity_palette() {
        fn naive_picks_black(c: Rgba) -> bool {
            u32::from(c.r) * 299 + u32::from(c.g) * 587 + u32::from(c.b) * 114 > 128_000
        }

        // d2b's own default accent, used in the shipping rail.
        let orange = Rgba::rgb(0xff, 0xa5, 0x00);
        assert!(naive_picks_black(orange));
        assert_eq!(readable_on(orange), Rgba::BLACK);

        // A mid green where the two rules part company: naive luma says the
        // colour is "dark" and asks for white text, but WCAG luminance shows
        // black is the higher-contrast choice by a wide margin.
        let green = Rgba::rgb(0x00, 0x9e, 0x60);
        assert!(!naive_picks_black(green), "naive luma would choose white");
        assert_eq!(
            readable_on(green),
            Rgba::BLACK,
            "WCAG luminance prefers black"
        );
        assert!(contrast_ratio(Rgba::BLACK, green) > contrast_ratio(Rgba::WHITE, green));
    }

    /// The true bound, computed rather than assumed: choosing the better of
    /// black/white always clears 4.5:1, but only just.
    ///
    /// The worst case is analytic. Contrast against black is (L+0.05)/0.05 and
    /// against white is 1.05/(L+0.05); they cross where (L+0.05)^2 = 0.0525,
    /// i.e. L* = sqrt(0.0525) - 0.05 = 0.1791, giving 4.5826:1. A fine sweep of
    /// the colour cube confirms no background does worse.
    #[test]
    fn best_of_black_or_white_always_clears_body_text_but_barely() {
        let l_star = 0.0525_f64.sqrt() - 0.05;
        let analytic = (l_star + 0.05) / 0.05;
        assert!((analytic - 4.5826).abs() < 1e-3, "analytic bound {analytic}");
        assert!(analytic > CONTRAST_TEXT_AA);

        let mut worst = f64::MAX;
        for r in (0..=255).step_by(5) {
            for g in (0..=255).step_by(5) {
                for b in (0..=255).step_by(5) {
                    let bg = Rgba::rgb(r, g, b);
                    worst = worst.min(contrast_ratio(readable_on(bg), bg));
                }
            }
        }
        assert!(worst >= CONTRAST_TEXT_AA, "worst sampled ratio {worst}");
        assert!(
            worst < 4.7,
            "the margin is thin ({worst}); this is why chrome does not put \
             small text directly on an arbitrary accent fill"
        );
    }

    /// The concrete cost of the shipping proxy's naive luma threshold: it does
    /// not merely disagree with WCAG, it produces contrast as low as ~1.94:1,
    /// far below the 4.5:1 floor, on ordinary saturated colours.
    #[test]
    fn naive_luma_produces_real_wcag_failures() {
        fn naive_picks_black(c: Rgba) -> bool {
            u32::from(c.r) * 299 + u32::from(c.g) * 587 + u32::from(c.b) * 114 > 128_000
        }

        // Sitting just under the naive threshold, this bright green gets white
        // text from the naive rule and is nearly illegible.
        let green = Rgba::rgb(4, 216, 0);
        assert!(!naive_picks_black(green), "naive luma chooses white here");
        let naive_ratio = contrast_ratio(Rgba::WHITE, green);
        assert!(
            naive_ratio < 2.0,
            "expected a severe failure, got {naive_ratio}"
        );
        // The correct rule recovers a compliant choice on the same colour.
        assert_eq!(readable_on(green), Rgba::BLACK);
        assert!(contrast_ratio(readable_on(green), green) >= CONTRAST_TEXT_AA);
    }

    #[test]
    fn enforce_contrast_reports_impossible_targets() {
        // Against a mid-tone background, a floor above the analytic maximum
        // for that background is unreachable and must be reported, not faked.
        let bg = Rgba::rgb(0x80, 0x80, 0x80);
        let best = contrast_ratio(readable_on(bg), bg);
        assert!(enforce_contrast(Rgba::WHITE, bg, best + 1.0).is_none());
        assert!(enforce_contrast(Rgba::WHITE, bg, best - 0.5).is_some());
    }

    #[test]
    fn enforce_contrast_reaches_the_floor_when_possible() {
        let bg = Rgba::rgb(0x10, 0x10, 0x14);
        let dim = Rgba::rgb(0x40, 0x40, 0x48);
        assert!(!meets_contrast(dim, bg, CONTRAST_TEXT_AA));
        let fixed = enforce_contrast(dim, bg, CONTRAST_TEXT_AA).expect("reachable");
        assert!(meets_contrast(fixed, bg, CONTRAST_TEXT_AA));
        // And it should not overshoot all the way to pure white.
        assert_ne!(fixed, Rgba::WHITE);
    }

    #[test]
    fn enforce_contrast_is_identity_when_already_passing() {
        let bg = Rgba::rgb(0x10, 0x10, 0x14);
        assert_eq!(
            enforce_contrast(Rgba::WHITE, bg, CONTRAST_TEXT_AA),
            Some(Rgba::WHITE)
        );
    }

    #[test]
    fn parse_hex_roundtrip_and_rejection() {
        assert_eq!(Rgba::parse_hex("#ffa500").unwrap(), Rgba::rgb(255, 165, 0));
        assert_eq!(
            Rgba::parse_hex("#ffa50080").unwrap(),
            Rgba::rgba(255, 165, 0, 128)
        );
        assert_eq!(Rgba::rgb(255, 165, 0).to_hex(), "#ffa500");
        for bad in ["ffa500", "#fff", "#gggggg", "#ffa5000"] {
            assert!(Rgba::parse_hex(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn over_composites_straight_alpha() {
        let half_white = Rgba::rgba(255, 255, 255, 128);
        let out = half_white.over(Rgba::BLACK);
        assert_eq!(out.a, 255);
        assert!((127..=129).contains(&out.r), "got {}", out.r);
        assert_eq!(Rgba::TRANSPARENT.over(Rgba::BLACK), Rgba::BLACK);
        assert_eq!(Rgba::WHITE.over(Rgba::BLACK), Rgba::WHITE);
    }

    #[test]
    fn grayscale_delta_flags_indistinguishable_identity_pairs() {
        // Red and green of similar luma collapse in grayscale: this is exactly
        // the monochrome-display failure the design must avoid.
        let red = Rgba::rgb(0xd0, 0x40, 0x40);
        let green = Rgba::rgb(0x40, 0xa0, 0x40);
        assert!(
            grayscale_delta(red, green) < 0.05,
            "expected a collapse, got {}",
            grayscale_delta(red, green)
        );
        // A light/dark pair survives.
        assert!(grayscale_delta(Rgba::rgb(0xe8, 0xe8, 0xe8), Rgba::rgb(0x30, 0x30, 0x30)) > 0.3);
    }

    #[test]
    fn argb8888_byte_order() {
        assert_eq!(Rgba::rgb(0x11, 0x22, 0x33).argb8888(), [0x33, 0x22, 0x11, 255]);
    }

    /// wl_shm's Argb8888 is premultiplied. Submitting straight alpha makes
    /// antialiased edges render as bright fringes, which is exactly what a
    /// white halo on a rounded corner looks like.
    #[test]
    fn premultiplied_conversion_scales_colour_by_alpha() {
        // Opaque values are unchanged.
        let opaque = Rgba::rgb(0x11, 0x22, 0x33);
        assert_eq!(opaque.argb8888_premultiplied(), opaque.argb8888());

        // Fully transparent collapses to zero, not to its colour.
        assert_eq!(
            Rgba::rgba(0xff, 0xff, 0xff, 0).argb8888_premultiplied(),
            [0, 0, 0, 0]
        );

        // Half-transparent white must not stay full-brightness.
        let half = Rgba::rgba(0xff, 0xff, 0xff, 0x80);
        let straight = half.argb8888();
        let pre = half.argb8888_premultiplied();
        assert_eq!(straight[0], 0xff, "straight alpha keeps full brightness");
        assert!(pre[0] < 0x82 && pre[0] > 0x7e, "premultiplied halves it: {pre:?}");
        assert_eq!(pre[3], 0x80, "alpha itself is untouched");
    }
}
