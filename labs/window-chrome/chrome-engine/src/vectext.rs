//! Text as vector outlines.
//!
//! The bitmap path costs about 19 MB of resident memory for a single face,
//! almost all of it rasterizer cache, which is a poor trade for a strip of
//! chrome that draws a handful of short words. Outlines are read straight from
//! the font and filled as paths by the same rasterizer that draws the tab, so
//! text costs the parsed face and nothing else, and scales without resampling.

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};
use ttf_parser::{Face, GlyphId, OutlineBuilder};

use crate::color::Rgba;

/// A parsed face plus the metrics chrome needs.
pub struct VectorFont<'a> {
    face: Face<'a>,
    units_per_em: f32,
}

/// Collects glyph outlines into a tiny-skia path, flipping the Y axis: fonts
/// grow upward from the baseline, the canvas grows downward.
struct PathSink {
    pb: PathBuilder,
    x: f32,
    y: f32,
    scale: f32,
}

impl OutlineBuilder for PathSink {
    fn move_to(&mut self, x: f32, y: f32) {
        self.pb.move_to(self.x + x * self.scale, self.y - y * self.scale);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.pb.line_to(self.x + x * self.scale, self.y - y * self.scale);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.pb.quad_to(
            self.x + x1 * self.scale,
            self.y - y1 * self.scale,
            self.x + x * self.scale,
            self.y - y * self.scale,
        );
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.pb.cubic_to(
            self.x + x1 * self.scale,
            self.y - y1 * self.scale,
            self.x + x2 * self.scale,
            self.y - y2 * self.scale,
            self.x + x * self.scale,
            self.y - y * self.scale,
        );
    }
    fn close(&mut self) {
        self.pb.close();
    }
}

impl<'a> VectorFont<'a> {
    pub fn from_bytes(data: &'a [u8]) -> Result<Self, String> {
        let face = Face::parse(data, 0).map_err(|e| format!("failed to parse font: {e}"))?;
        let units_per_em = f32::from(face.units_per_em());
        if units_per_em <= 0.0 {
            return Err("font reports no units per em".to_owned());
        }
        Ok(Self { face, units_per_em })
    }

    fn scale_for(&self, px_size: f32) -> f32 {
        px_size / self.units_per_em
    }

    fn glyph(&self, ch: char) -> Option<GlyphId> {
        self.face.glyph_index(ch)
    }

    /// Advance width of `text` at `px_size`, including inter-glyph tracking.
    pub fn measure(&self, text: &str, px_size: f32, tracking: f32) -> f32 {
        let scale = self.scale_for(px_size);
        let count = text.chars().count();
        let mut width = 0.0;
        for (i, ch) in text.chars().enumerate() {
            if let Some(gid) = self.glyph(ch) {
                if let Some(adv) = self.face.glyph_hor_advance(gid) {
                    width += f32::from(adv) * scale;
                }
            }
            if i + 1 < count {
                width += tracking;
            }
        }
        width
    }

    /// Distance from the baseline to the top of a capital letter, which is what
    /// chrome centres on rather than the full ascender.
    pub fn cap_height(&self, px_size: f32) -> f32 {
        let scale = self.scale_for(px_size);
        self.face
            .capital_height()
            .map(|v| f32::from(v) * scale)
            .unwrap_or_else(|| f32::from(self.face.ascender()) * scale * 0.7)
    }

    pub fn ascender(&self, px_size: f32) -> f32 {
        f32::from(self.face.ascender()) * self.scale_for(px_size)
    }

    /// Fill `text` with its baseline at `baseline_y`, starting at `origin_x`.
    pub fn draw(
        &self,
        pixmap: &mut Pixmap,
        text: &str,
        px_size: f32,
        tracking: f32,
        origin_x: f32,
        baseline_y: f32,
        color: Rgba,
    ) -> f32 {
        let scale = self.scale_for(px_size);
        let mut pen = origin_x;
        let mut sink = PathSink {
            pb: PathBuilder::new(),
            x: 0.0,
            y: 0.0,
            scale,
        };
        for ch in text.chars() {
            let Some(gid) = self.glyph(ch) else { continue };
            sink.x = pen;
            sink.y = baseline_y;
            self.face.outline_glyph(gid, &mut sink);
            if let Some(adv) = self.face.glyph_hor_advance(gid) {
                pen += f32::from(adv) * scale;
            }
            pen += tracking;
        }
        if let Some(path) = sink.pb.finish() {
            let mut paint = Paint::default();
            paint.set_color_rgba8(color.r, color.g, color.b, color.a);
            paint.anti_alias = true;
            pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
        pen - origin_x
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PROTOTYPE_FONT;

    fn font() -> VectorFont<'static> {
        VectorFont::from_bytes(PROTOTYPE_FONT).expect("bundled font parses")
    }

    #[test]
    fn measure_scales_with_size_and_tracking() {
        let f = font();
        let small = f.measure("Work", 12.0, 0.0);
        let large = f.measure("Work", 24.0, 0.0);
        assert!(small > 0.0);
        assert!(large > small * 1.9, "24px should be about double 12px");
        assert!(f.measure("Work", 12.0, 2.0) > small);
    }

    #[test]
    fn cap_height_is_below_the_ascender() {
        let f = font();
        let cap = f.cap_height(14.0);
        assert!(cap > 0.0);
        assert!(cap <= f.ascender(14.0));
    }

    #[test]
    fn drawing_puts_ink_on_the_canvas() {
        let f = font();
        let mut pm = Pixmap::new(120, 30).unwrap();
        let advance = f.draw(&mut pm, "Work", 14.0, 0.0, 4.0, 20.0, Rgba::WHITE);
        assert!(advance > 0.0);
        let inked = pm.pixels().iter().filter(|p| p.alpha() > 0).count();
        assert!(inked > 40, "expected glyph coverage, got {inked} pixels");
    }

    #[test]
    fn text_stays_within_its_measured_advance() {
        let f = font();
        let px = 14.0;
        let text = "Work";
        let width = f.measure(text, px, 0.0);
        let mut pm = Pixmap::new(200, 40).unwrap();
        f.draw(&mut pm, text, px, 0.0, 10.0, 28.0, Rgba::WHITE);
        // Nothing is drawn past the advance, so layout that reserves the
        // measured width cannot clip the label.
        let limit = (10.0 + width).ceil() as u32 + 1;
        for y in 0..40u32 {
            for x in limit..200u32 {
                assert_eq!(
                    pm.pixel(x, y).unwrap().alpha(),
                    0,
                    "ink at {x},{y} beyond the measured advance"
                );
            }
        }
    }

    #[test]
    fn unknown_glyphs_are_skipped_without_panicking() {
        let f = font();
        let mut pm = Pixmap::new(60, 30).unwrap();
        // A private-use codepoint the face will not have.
        f.draw(&mut pm, "\u{E000}", 14.0, 0.0, 2.0, 20.0, Rgba::WHITE);
    }
}
