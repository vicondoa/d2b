//! Text measurement, abstracted away from font rasterization.
//!
//! Layout correctness does not depend on which font is loaded, so the proof
//! measures with a deterministic metric rather than pulling in a rasterizer.
//! The shipped renderer implements this trait over a real font face; the
//! properties proved here hold for any implementation that is monotonic in
//! string length, which every text shaper is.

/// Something that can report the advance width of a string.
pub trait Measure {
    /// Advance width of `text` at `px` with `tracking` extra per glyph.
    fn measure(&self, text: &str, px: f32, tracking: f32) -> f32;
}

/// A deterministic metric: every glyph is 0.55 em wide.
///
/// Chosen because it is close to a real sans-serif average, so the numbers in
/// failing tests are recognisable, and because being exactly proportional to
/// character count makes the scaling properties checkable by hand.
#[derive(Debug, Clone, Copy)]
pub struct FixedMetric;

pub const GLYPH_EM: f32 = 0.55;

impl Measure for FixedMetric {
    fn measure(&self, text: &str, px: f32, tracking: f32) -> f32 {
        let n = text.chars().count() as f32;
        n * (px * GLYPH_EM + tracking)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measurement_is_proportional_to_length() {
        let m = FixedMetric;
        assert!((m.measure("ab", 12.0, 0.0) - 2.0 * m.measure("a", 12.0, 0.0)).abs() < 1e-4);
    }

    #[test]
    fn measurement_scales_with_size() {
        let m = FixedMetric;
        let a = m.measure("work", 12.0, 0.0);
        let b = m.measure("work", 24.0, 0.0);
        assert!((b - 2.0 * a).abs() < 1e-4);
    }

    #[test]
    fn tracking_adds_per_glyph() {
        let m = FixedMetric;
        let base = m.measure("work", 12.0, 0.0);
        let tracked = m.measure("work", 12.0, 2.0);
        assert!((tracked - base - 8.0).abs() < 1e-4);
    }

    #[test]
    fn the_empty_string_measures_zero() {
        assert_eq!(FixedMetric.measure("", 12.0, 1.0), 0.0);
    }
}
