//! Text rasterization for identity chrome.
//!
//! The shipping rail uses a 5x7 bitmap font rotated 90 degrees and stretched
//! 1x2. This module exists so labels are horizontal, real, and scale-aware:
//! glyphs are rasterized at the physical pixel size implied by the output
//! scale, never scaled up from a fixed bitmap.

use fontdue::{Font, FontSettings};

use crate::color::Rgba;

/// A font face plus the metrics chrome needs.
pub struct TextRenderer {
    font: Font,
}

/// One laid-out glyph, positioned in physical pixels relative to the text origin.
#[derive(Debug, Clone)]
pub struct PositionedGlyph {
    pub ch: char,
    pub x: i32,
    pub y: i32,
    pub width: usize,
    pub height: usize,
    /// Coverage mask, one byte per pixel, row-major.
    pub coverage: Vec<u8>,
}

/// Measured extents of a laid-out run, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextMetrics {
    pub width: u32,
    pub ascent: u32,
    pub descent: u32,
}

impl TextMetrics {
    pub fn height(self) -> u32 {
        self.ascent + self.descent
    }
}

impl TextRenderer {
    /// Load a face from TTF/OTF bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        let font = Font::from_bytes(data, FontSettings::default())
            .map_err(|e| format!("failed to parse font: {e}"))?;
        Ok(Self { font })
    }

    /// Horizontal advance of `text` at `px_size` physical pixels.
    pub fn measure(&self, text: &str, px_size: f32, tracking: f32) -> TextMetrics {
        let mut width = 0.0_f32;
        let mut ascent = 0.0_f32;
        let mut descent = 0.0_f32;
        for (i, ch) in text.chars().enumerate() {
            let m = self.font.metrics(ch, px_size);
            width += m.advance_width;
            if i + 1 < text.chars().count() {
                width += tracking;
            }
            let top = m.bounds.height + m.bounds.ymin;
            ascent = ascent.max(top);
            descent = descent.max(-m.bounds.ymin);
        }
        TextMetrics {
            width: width.ceil().max(0.0) as u32,
            ascent: ascent.ceil().max(0.0) as u32,
            descent: descent.ceil().max(0.0) as u32,
        }
    }

    /// Lay out `text` with its baseline at `baseline_y`, starting at `origin_x`.
    pub fn layout(
        &self,
        text: &str,
        px_size: f32,
        tracking: f32,
        origin_x: i32,
        baseline_y: i32,
    ) -> Vec<PositionedGlyph> {
        let mut pen = origin_x as f32;
        let mut out = Vec::new();
        for ch in text.chars() {
            let (metrics, coverage) = self.font.rasterize(ch, px_size);
            if metrics.width > 0 && metrics.height > 0 {
                out.push(PositionedGlyph {
                    ch,
                    x: (pen + metrics.xmin as f32).round() as i32,
                    // fontdue's ymin is the offset of the glyph's bottom from
                    // the baseline, positive upward, so the top edge sits at
                    // baseline - (height + ymin).
                    y: baseline_y - (metrics.height as i32 + metrics.ymin),
                    width: metrics.width,
                    height: metrics.height,
                    coverage,
                });
            }
            pen += metrics.advance_width + tracking;
        }
        out
    }

    /// Truncate `text` with a trailing ellipsis so it fits `max_width`.
    ///
    /// Returns the original string when it already fits. Never returns a string
    /// wider than `max_width` unless even a lone ellipsis does not fit, in
    /// which case it returns an empty string: chrome must not overflow into
    /// neighbouring pixels.
    pub fn ellipsize(&self, text: &str, px_size: f32, tracking: f32, max_width: u32) -> String {
        if self.measure(text, px_size, tracking).width <= max_width {
            return text.to_owned();
        }
        const ELLIPSIS: char = '\u{2026}';
        let ell_w = self
            .measure(&ELLIPSIS.to_string(), px_size, tracking)
            .width;
        if ell_w > max_width {
            return String::new();
        }
        let chars: Vec<char> = text.chars().collect();
        let mut best = String::new();
        for take in (0..chars.len()).rev() {
            let mut candidate: String = chars[..take].iter().collect();
            candidate.push(ELLIPSIS);
            if self.measure(&candidate, px_size, tracking).width <= max_width {
                best = candidate;
                break;
            }
        }
        if best.is_empty() {
            best.push(ELLIPSIS);
        }
        best
    }

    /// Split a dotted canonical label at its last delimiter so a long identity
    /// wraps semantically (`corp-workstation.work`) instead of mid-word.
    pub fn split_at_delimiter(text: &str) -> Option<(String, String)> {
        let idx = text.rfind('.')?;
        if idx == 0 || idx + 1 >= text.len() {
            return None;
        }
        Some((text[..idx].to_owned(), text[idx + 1..].to_owned()))
    }
}

/// Blend a glyph's coverage mask into an RGBA buffer.
pub fn blend_glyph(
    pixels: &mut [Rgba],
    buf_w: usize,
    buf_h: usize,
    glyph: &PositionedGlyph,
    color: Rgba,
) {
    for gy in 0..glyph.height {
        let py = glyph.y + gy as i32;
        if py < 0 || py as usize >= buf_h {
            continue;
        }
        for gx in 0..glyph.width {
            let px = glyph.x + gx as i32;
            if px < 0 || px as usize >= buf_w {
                continue;
            }
            let cov = glyph.coverage[gy * glyph.width + gx];
            if cov == 0 {
                continue;
            }
            let a = (u16::from(cov) * u16::from(color.a) / 255) as u8;
            let idx = py as usize * buf_w + px as usize;
            pixels[idx] = color.with_alpha(a).over(pixels[idx]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONT: &[u8] = include_bytes!("../assets/font.ttf");

    fn renderer() -> TextRenderer {
        TextRenderer::from_bytes(FONT).expect("bundled font must parse")
    }

    #[test]
    fn measure_scales_with_size() {
        let r = renderer();
        let small = r.measure("work", 12.0, 0.0).width;
        let large = r.measure("work", 24.0, 0.0).width;
        assert!(small > 0);
        assert!(
            large > small,
            "24px ({large}) should exceed 12px ({small})"
        );
    }

    #[test]
    fn measure_grows_with_tracking() {
        let r = renderer();
        let tight = r.measure("work", 13.0, 0.0).width;
        let loose = r.measure("work", 13.0, 2.0).width;
        assert!(loose > tight);
    }

    #[test]
    fn layout_produces_glyphs_with_coverage() {
        let r = renderer();
        let glyphs = r.layout("work", 13.0, 0.0, 0, 13);
        assert_eq!(glyphs.len(), 4, "every glyph in `work` has ink");
        for g in &glyphs {
            assert_eq!(g.coverage.len(), g.width * g.height);
            assert!(g.coverage.iter().any(|&c| c > 0), "glyph {} had no ink", g.ch);
        }
        // Glyphs must advance left to right.
        for pair in glyphs.windows(2) {
            assert!(pair[1].x >= pair[0].x, "glyphs must not run backwards");
        }
    }

    #[test]
    fn layout_skips_whitespace_without_ink() {
        let r = renderer();
        let glyphs = r.layout("a b", 13.0, 0.0, 0, 13);
        assert_eq!(glyphs.len(), 2, "the space contributes no glyph");
    }

    #[test]
    fn ellipsize_returns_original_when_it_fits() {
        let r = renderer();
        let wide = r.measure("work", 13.0, 0.0).width + 50;
        assert_eq!(r.ellipsize("work", 13.0, 0.0, wide), "work");
    }

    #[test]
    fn ellipsize_never_exceeds_the_budget() {
        let r = renderer();
        let long = "corp-workstation.work";
        for budget in [12_u32, 20, 40, 60, 80, 120] {
            let out = r.ellipsize(long, 13.0, 0.0, budget);
            let w = r.measure(&out, 13.0, 0.0).width;
            assert!(
                w <= budget || out.is_empty(),
                "budget {budget}: `{out}` measured {w}"
            );
        }
    }

    #[test]
    fn ellipsize_yields_empty_when_even_ellipsis_does_not_fit() {
        let r = renderer();
        assert_eq!(r.ellipsize("corp-workstation.work", 13.0, 0.0, 1), "");
    }

    #[test]
    fn split_at_delimiter_uses_the_realm_boundary() {
        assert_eq!(
            TextRenderer::split_at_delimiter("corp-workstation.work"),
            Some(("corp-workstation".to_owned(), "work".to_owned()))
        );
        assert_eq!(
            TextRenderer::split_at_delimiter("a.b.c"),
            Some(("a.b".to_owned(), "c".to_owned())),
            "split at the LAST delimiter so the realm stays whole"
        );
        assert_eq!(TextRenderer::split_at_delimiter("work"), None);
        assert_eq!(TextRenderer::split_at_delimiter(".work"), None);
        assert_eq!(TextRenderer::split_at_delimiter("work."), None);
    }

    #[test]
    fn blend_glyph_clips_at_buffer_edges() {
        let r = renderer();
        let mut buf = vec![Rgba::TRANSPARENT; 8 * 8];
        // Deliberately position partly off the top-left corner.
        let mut glyphs = r.layout("W", 13.0, 0.0, -3, 4);
        let g = glyphs.remove(0);
        blend_glyph(&mut buf, 8, 8, &g, Rgba::WHITE);
        // The point is that this does not panic and stays in bounds.
        assert_eq!(buf.len(), 64);
    }

    #[test]
    fn tracking_at_wcag_text_spacing_is_measurable() {
        // WCAG 1.4.12 requires surviving +0.12em letter-spacing; chrome must
        // size from measurement, so verify the measurement responds.
        let r = renderer();
        let px = 13.0_f32;
        let base = r.measure("corp-workstation.work", px, 0.0).width;
        let spaced = r.measure("corp-workstation.work", px, px * 0.12).width;
        assert!(spaced > base);
    }
}
