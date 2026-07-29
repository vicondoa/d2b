//! A tiny RGBA canvas with the primitives chrome needs, plus PNG output.

use std::{fs::File, io::BufWriter, path::Path};

use crate::color::Rgba;

pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Rgba>,
}

impl Canvas {
    pub fn new(width: usize, height: usize, fill: Rgba) -> Self {
        Self {
            width,
            height,
            pixels: vec![fill; width * height],
        }
    }

    pub fn get(&self, x: usize, y: usize) -> Rgba {
        self.pixels[y * self.width + x]
    }

    /// Alpha-composite one pixel, ignoring out-of-bounds writes.
    pub fn blend(&mut self, x: i32, y: i32, c: Rgba) {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return;
        }
        let i = y as usize * self.width + x as usize;
        self.pixels[i] = c.over(self.pixels[i]);
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, c: Rgba) {
        for dy in 0..h as i32 {
            for dx in 0..w as i32 {
                self.blend(x + dx, y + dy, c);
            }
        }
    }

    /// Rounded rectangle with analytic coverage antialiasing, so the shape is
    /// crisp at fractional scale instead of stair-stepped.
    pub fn fill_round_rect(&mut self, x: i32, y: i32, w: u32, h: u32, radius: f64, c: Rgba) {
        if w == 0 || h == 0 {
            return;
        }
        let r = radius.min(f64::from(w) / 2.0).min(f64::from(h) / 2.0).max(0.0);
        for dy in 0..h as i32 {
            for dx in 0..w as i32 {
                let cov = round_rect_coverage(f64::from(dx), f64::from(dy), f64::from(w), f64::from(h), r);
                if cov <= 0.0 {
                    continue;
                }
                let a = (f64::from(c.a) * cov).round().clamp(0.0, 255.0) as u8;
                self.blend(x + dx, y + dy, c.with_alpha(a));
            }
        }
    }

    /// Rounded rectangle with independent horizontal radii for the left and
    /// right corners and a shared vertical radius.
    ///
    /// This exists so an inset fill can share its arc centres with the shape it
    /// sits inside. A border with different insets per side cannot be offset by
    /// a circular arc; using an ellipse whose centre matches the outer arc's
    /// centre keeps the two contours parallel and lets the thickness taper
    /// smoothly instead of visibly diverging.
    pub fn fill_round_rect_xy(
        &mut self,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        rx_left: f64,
        rx_right: f64,
        ry: f64,
        c: Rgba,
    ) {
        if w == 0 || h == 0 {
            return;
        }
        let half_w = f64::from(w) / 2.0;
        let half_h = f64::from(h) / 2.0;
        let rxl = rx_left.clamp(0.0, half_w);
        let rxr = rx_right.clamp(0.0, half_w);
        let ry = ry.clamp(0.0, half_h);
        for dy in 0..h as i32 {
            for dx in 0..w as i32 {
                let cov = ellipse_rect_coverage(
                    f64::from(dx),
                    f64::from(dy),
                    f64::from(w),
                    f64::from(h),
                    rxl,
                    rxr,
                    ry,
                );
                if cov <= 0.0 {
                    continue;
                }
                let a = (f64::from(c.a) * cov).round().clamp(0.0, 255.0) as u8;
                self.blend(x + dx, y + dy, c.with_alpha(a));
            }
        }
    }

    /// A 1px hairline rectangle outline.
    pub fn stroke_rect(&mut self, x: i32, y: i32, w: u32, h: u32, c: Rgba) {
        if w == 0 || h == 0 {
            return;
        }
        self.fill_rect(x, y, w, 1, c);
        self.fill_rect(x, y + h as i32 - 1, w, 1, c);
        self.fill_rect(x, y, 1, h, c);
        self.fill_rect(x + w as i32 - 1, y, 1, h, c);
    }

    /// A 1px outline that follows a rounded rectangle, so a border traces the
    /// same corners as the fill instead of cutting across them.
    pub fn stroke_round_rect(&mut self, x: i32, y: i32, w: u32, h: u32, radius: f64, c: Rgba) {
        if w == 0 || h == 0 {
            return;
        }
        let r = radius.min(f64::from(w) / 2.0).min(f64::from(h) / 2.0).max(0.0);
        if r <= 0.0 {
            self.stroke_rect(x, y, w, h, c);
            return;
        }
        // Coverage of the shape minus coverage of a shape inset by one pixel
        // gives an antialiased one-pixel ring.
        for dy in 0..h as i32 {
            for dx in 0..w as i32 {
                let outer = round_rect_coverage(
                    f64::from(dx),
                    f64::from(dy),
                    f64::from(w),
                    f64::from(h),
                    r,
                );
                if outer <= 0.0 {
                    continue;
                }
                let inner = round_rect_coverage(
                    f64::from(dx) - 1.0,
                    f64::from(dy) - 1.0,
                    f64::from(w) - 2.0,
                    f64::from(h) - 2.0,
                    (r - 1.0).max(0.0),
                );
                let ring = (outer - inner).clamp(0.0, 1.0);
                if ring <= 0.0 {
                    continue;
                }
                let a = (f64::from(c.a) * ring).round().clamp(0.0, 255.0) as u8;
                self.blend(x + dx, y + dy, c.with_alpha(a));
            }
        }
    }

    /// Composite `other` at (x, y).
    pub fn draw(&mut self, other: &Canvas, x: i32, y: i32) {
        for oy in 0..other.height {
            for ox in 0..other.width {
                self.blend(x + ox as i32, y + oy as i32, other.get(ox, oy));
            }
        }
    }

    /// Model a monochrome display.
    pub fn to_grayscale(&self) -> Canvas {
        Canvas {
            width: self.width,
            height: self.height,
            pixels: self.pixels.iter().map(|p| p.to_grayscale()).collect(),
        }
    }

    pub fn to_rgba_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 4);
        for p in &self.pixels {
            out.extend_from_slice(&[p.r, p.g, p.b, p.a]);
        }
        out
    }

    /// ARGB8888 little-endian, the wl_shm layout the proxy attaches.
    pub fn to_argb8888(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() * 4);
        for p in &self.pixels {
            out.extend_from_slice(&p.argb8888());
        }
        out
    }

    pub fn write_png(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let file = File::create(path.as_ref())
            .map_err(|e| format!("create {}: {e}", path.as_ref().display()))?;
        let mut encoder = png::Encoder::new(
            BufWriter::new(file),
            self.width as u32,
            self.height as u32,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Best);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("png header: {e}"))?;
        writer
            .write_image_data(&self.to_rgba_bytes())
            .map_err(|e| format!("png data: {e}"))
    }
}

/// Approximate pixel coverage of a rounded rect, sampled 4x4 within the pixel.
fn round_rect_coverage(px: f64, py: f64, w: f64, h: f64, r: f64) -> f64 {
    if r <= 0.0 {
        return 1.0;
    }
    const S: usize = 4;
    let mut hits = 0.0;
    for sy in 0..S {
        for sx in 0..S {
            let x = px + (sx as f64 + 0.5) / S as f64;
            let y = py + (sy as f64 + 0.5) / S as f64;
            if inside_round_rect(x, y, w, h, r) {
                hits += 1.0;
            }
        }
    }
    hits / (S * S) as f64
}

fn inside_round_rect(x: f64, y: f64, w: f64, h: f64, r: f64) -> bool {
    inside_ellipse_rect(x, y, w, h, r, r, r)
}

/// Coverage for a rectangle whose corners are elliptical, with independent
/// horizontal radii on the left and right.
fn ellipse_rect_coverage(
    px: f64,
    py: f64,
    w: f64,
    h: f64,
    rx_left: f64,
    rx_right: f64,
    ry: f64,
) -> f64 {
    if rx_left <= 0.0 && rx_right <= 0.0 && ry <= 0.0 {
        return if px >= 0.0 && py >= 0.0 && px < w && py < h {
            1.0
        } else {
            0.0
        };
    }
    const S: usize = 4;
    let mut hits = 0.0;
    for sy in 0..S {
        for sx in 0..S {
            let x = px + (sx as f64 + 0.5) / S as f64;
            let y = py + (sy as f64 + 0.5) / S as f64;
            if inside_ellipse_rect(x, y, w, h, rx_left, rx_right, ry) {
                hits += 1.0;
            }
        }
    }
    hits / (S * S) as f64
}

fn inside_ellipse_rect(x: f64, y: f64, w: f64, h: f64, rx_left: f64, rx_right: f64, ry: f64) -> bool {
    if x < 0.0 || y < 0.0 || x > w || y > h {
        return false;
    }
    // Pick the corner this point belongs to, if any.
    let (cx, rx) = if x < rx_left {
        (rx_left, rx_left)
    } else if x > w - rx_right {
        (w - rx_right, rx_right)
    } else {
        return y >= 0.0 && y <= h;
    };
    let cy = if y < ry {
        ry
    } else if y > h - ry {
        h - ry
    } else {
        return true;
    };
    if rx <= 0.0 || ry <= 0.0 {
        return true;
    }
    let nx = (x - cx) / rx;
    let ny = (y - cy) / ry;
    nx * nx + ny * ny <= 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_rect_writes_only_inside() {
        let mut c = Canvas::new(10, 10, Rgba::TRANSPARENT);
        c.fill_rect(2, 3, 4, 2, Rgba::WHITE);
        assert_eq!(c.get(2, 3), Rgba::WHITE);
        assert_eq!(c.get(5, 4), Rgba::WHITE);
        assert_eq!(c.get(6, 4), Rgba::TRANSPARENT, "right edge exclusive");
        assert_eq!(c.get(2, 5), Rgba::TRANSPARENT, "bottom edge exclusive");
        assert_eq!(c.get(1, 3), Rgba::TRANSPARENT);
    }

    #[test]
    fn out_of_bounds_writes_are_ignored() {
        let mut c = Canvas::new(4, 4, Rgba::TRANSPARENT);
        c.fill_rect(-10, -10, 100, 100, Rgba::WHITE);
        // Must not panic, and must have filled the visible area.
        assert_eq!(c.get(0, 0), Rgba::WHITE);
        assert_eq!(c.get(3, 3), Rgba::WHITE);
    }

    #[test]
    fn round_rect_clears_its_corners_and_fills_its_centre() {
        let mut c = Canvas::new(20, 20, Rgba::TRANSPARENT);
        c.fill_round_rect(0, 0, 20, 20, 6.0, Rgba::WHITE);
        assert_eq!(c.get(10, 10), Rgba::WHITE, "centre is solid");
        assert_eq!(c.get(0, 0).a, 0, "corner pixel is empty");
        assert_eq!(c.get(19, 0).a, 0);
        assert_eq!(c.get(0, 19).a, 0);
        assert_eq!(c.get(19, 19).a, 0);
    }

    #[test]
    fn round_rect_corner_is_antialiased_not_binary() {
        let mut c = Canvas::new(20, 20, Rgba::TRANSPARENT);
        c.fill_round_rect(0, 0, 20, 20, 6.0, Rgba::WHITE);
        // Somewhere along the corner arc there must be partial coverage.
        let partial = (0..8)
            .flat_map(|y| (0..8).map(move |x| (x, y)))
            .any(|(x, y)| {
                let a = c.get(x, y).a;
                a > 0 && a < 255
            });
        assert!(partial, "corner should be antialiased");
    }

    #[test]
    fn zero_radius_round_rect_is_a_plain_rect() {
        let mut c = Canvas::new(8, 8, Rgba::TRANSPARENT);
        c.fill_round_rect(0, 0, 8, 8, 0.0, Rgba::WHITE);
        assert_eq!(c.get(0, 0), Rgba::WHITE);
        assert_eq!(c.get(7, 7), Rgba::WHITE);
    }

    #[test]
    fn stroke_rect_draws_only_the_perimeter() {
        let mut c = Canvas::new(10, 10, Rgba::TRANSPARENT);
        c.stroke_rect(1, 1, 6, 6, Rgba::WHITE);
        assert_eq!(c.get(1, 1), Rgba::WHITE);
        assert_eq!(c.get(6, 6), Rgba::WHITE);
        assert_eq!(c.get(3, 3), Rgba::TRANSPARENT, "interior stays empty");
    }

    #[test]
    fn argb8888_layout_matches_wl_shm() {
        let mut c = Canvas::new(1, 1, Rgba::TRANSPARENT);
        c.fill_rect(0, 0, 1, 1, Rgba::rgb(0x11, 0x22, 0x33));
        assert_eq!(c.to_argb8888(), vec![0x33, 0x22, 0x11, 0xff]);
    }

    #[test]
    fn grayscale_collapses_hue_but_keeps_alpha() {
        let mut c = Canvas::new(2, 1, Rgba::TRANSPARENT);
        c.fill_rect(0, 0, 1, 1, Rgba::rgb(255, 0, 0));
        let g = c.to_grayscale();
        let p = g.get(0, 0);
        assert_eq!(p.r, p.g);
        assert_eq!(p.g, p.b);
        assert_eq!(g.get(1, 0).a, 0);
    }

    #[test]
    fn draw_composites_a_sub_canvas() {
        let mut base = Canvas::new(8, 8, Rgba::BLACK);
        let mut chip = Canvas::new(2, 2, Rgba::TRANSPARENT);
        chip.fill_rect(0, 0, 2, 2, Rgba::WHITE);
        base.draw(&chip, 3, 3);
        assert_eq!(base.get(3, 3), Rgba::WHITE);
        assert_eq!(base.get(2, 3), Rgba::BLACK);
    }
}
