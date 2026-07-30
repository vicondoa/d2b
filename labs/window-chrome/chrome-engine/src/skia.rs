//! Vector rendering for the identity tab.
//!
//! This replaces hand-written pixel loops with a real 2D rasterizer. That is
//! not only tidier: three of the defects found while iterating on this design
//! were direct consequences of hand-rolling. Premultiplied alpha was applied at
//! the wrong boundary and produced white fringes; concentric corners were
//! approximated with ellipses that could not stay parallel; and a damage
//! rectangle kept the axes of an older layout. A rasterizer removes the first
//! two categories entirely - `Pixmap` is premultiplied by definition, and a
//! stroked path is concentric with its fill by construction.
//!
//! Why not the toolkit `d2b-wlcontrol` uses: Quickshell renders its own
//! layer-shell surface from a separate process. The identity tab has to live on
//! a `wl_subsurface` of the proxy's wrapper toplevel so it tracks the window
//! through niri's scrolling layout, and one process cannot render into
//! another's subsurface. Layer-shell surfaces additionally cannot follow a
//! window at all; they anchor to screen edges. The toolkit is right for a
//! panel and unusable for window-attached chrome.

use tiny_skia::{
    FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, PixmapRef, Rect, Stroke, Transform,
};

use crate::{
    color::Rgba,
    text::{TextRenderer, TextMetrics},
    variant::{Action, ChromeSpec},
};

/// Circle-to-cubic constant: the arc control-point offset that approximates a
/// quarter circle to within a fraction of a pixel.
const KAPPA: f32 = 0.552_284_75;

fn paint_of(c: Rgba) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(c.r, c.g, c.b, c.a);
    paint.anti_alias = true;
    paint
}

/// Build a rounded-rectangle path. Corners are real arcs, so a stroke of this
/// path is concentric with a fill of it - which is what keeps the accent border
/// parallel to the card edge no matter the radius.
pub fn rounded_rect_path(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
) -> Option<tiny_skia::Path> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut pb = PathBuilder::new();
    if r <= 0.0 {
        pb.push_rect(Rect::from_xywh(x, y, w, h)?);
        return pb.finish();
    }
    let k = r * KAPPA;
    let (l, t, rt, b) = (x, y, x + w, y + h);

    pb.move_to(l + r, t);
    pb.line_to(rt - r, t);
    pb.cubic_to(rt - r + k, t, rt, t + r - k, rt, t + r);
    pb.line_to(rt, b - r);
    pb.cubic_to(rt, b - r + k, rt - r + k, b, rt - r, b);
    pb.line_to(l + r, b);
    pb.cubic_to(l + r - k, b, l, b - r + k, l, b - r);
    pb.line_to(l, t + r);
    pb.cubic_to(l, t + r - k, l + r - k, t, l + r, t);
    pb.close();
    pb.finish()
}

/// Everything the renderer needs, resolved in physical pixels.
pub struct TabGeometry {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    /// Width of the solid accent bar on the left edge.
    pub bar: f32,
    /// Width of the accent hairline on the other edges.
    pub hairline: f32,
}

/// Draw the tab into `pixmap`. Shapes only; text and icons are layered on top.
pub fn draw_tab_frame(
    pixmap: &mut Pixmap,
    geo: &TabGeometry,
    fill: Rgba,
    accent: Rgba,
    hairline_mix: f64,
) {
    let Some(outer) = rounded_rect_path(geo.x, geo.y, geo.width, geo.height, geo.radius) else {
        return;
    };

    // The card.
    pixmap.fill_path(
        &outer,
        &paint_of(fill),
        FillRule::Winding,
        Transform::identity(),
        None,
    );

    // The hairline, stroked on the card's own path. A stroke is centred on the
    // path, so insetting by half its width keeps it inside the card and exactly
    // concentric with it.
    let inset = geo.hairline / 2.0;
    if let Some(border) = rounded_rect_path(
        geo.x + inset,
        geo.y + inset,
        geo.width - geo.hairline,
        geo.height - geo.hairline,
        (geo.radius - inset).max(0.0),
    ) {
        let mut stroke = Stroke::default();
        stroke.width = geo.hairline;
        pixmap.stroke_path(
            &border,
            &paint_of(accent.mix(fill, hairline_mix)),
            &stroke,
            Transform::identity(),
            None,
        );
    }

    // The accent bar: the card's own shape, clipped to its left columns, so the
    // bar follows the card's curve on its left and is straight on its right.
    if geo.bar > 0.0 {
        let mut bar_pb = PathBuilder::new();
        if let Some(clip) = Rect::from_xywh(geo.x, geo.y, geo.bar, geo.height) {
            bar_pb.push_rect(clip);
        }
        if let Some(clip_path) = bar_pb.finish() {
            let mut mask = tiny_skia::Mask::new(pixmap.width(), pixmap.height()).unwrap();
            mask.fill_path(&clip_path, FillRule::Winding, true, Transform::identity());
            pixmap.fill_path(
                &outer,
                &paint_of(accent),
                FillRule::Winding,
                Transform::identity(),
                Some(&mask),
            );
        }
    }
}

/// Blit a font glyph coverage mask. Text shaping stays with the font crate;
/// only compositing happens here.
pub fn draw_text(
    pixmap: &mut Pixmap,
    fonts: &TextRenderer,
    text: &str,
    px_size: f32,
    tracking: f32,
    origin_x: f32,
    baseline_y: f32,
    color: Rgba,
) -> TextMetrics {
    let metrics = fonts.measure(text, px_size, tracking);
    for glyph in fonts.layout(text, px_size, tracking, origin_x as i32, baseline_y as i32) {
        if glyph.width == 0 || glyph.height == 0 {
            continue;
        }
        let Some(mut stamp) = Pixmap::new(glyph.width as u32, glyph.height as u32) else {
            continue;
        };
        for (i, cov) in glyph.coverage.iter().enumerate() {
            if *cov == 0 {
                continue;
            }
            let a = (u16::from(*cov) * u16::from(color.a) / 255) as u8;
            // Pixmap data is premultiplied; scale the colour by coverage.
            let mul = |c: u8| ((u16::from(c) * u16::from(a)) / 255) as u8;
            let px = &mut stamp.pixels_mut()[i];
            *px = tiny_skia::PremultipliedColorU8::from_rgba(
                mul(color.r),
                mul(color.g),
                mul(color.b),
                a,
            )
            .unwrap_or_else(|| tiny_skia::PremultipliedColorU8::from_rgba(0, 0, 0, 0).unwrap());
        }
        pixmap.draw_pixmap(
            glyph.x,
            glyph.y,
            PixmapRef::from_bytes(stamp.data(), stamp.width(), stamp.height())
                .unwrap_or_else(|| stamp.as_ref()),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
    metrics
}

/// Stroke-based action icons. Replaces hand-plotted rectangles with paths, so
/// the shapes stay proportional at any size and antialias correctly.
pub fn draw_action_icon(
    pixmap: &mut Pixmap,
    action: Action,
    x: f32,
    y: f32,
    size: f32,
    color: Rgba,
) {
    let s = size / 20.0;
    let p = |v: f32| v * s;
    let mut stroke = Stroke::default();
    stroke.width = (1.6 * s).max(1.0);
    stroke.line_cap = tiny_skia::LineCap::Round;
    stroke.line_join = tiny_skia::LineJoin::Round;
    let paint = paint_of(color);
    let mut pb = PathBuilder::new();

    match action {
        Action::Terminal => {
            if let Some(frame) = rounded_rect_path(x + p(2.0), y + p(3.0), p(16.0), p(14.0), p(2.5))
            {
                pixmap.stroke_path(&frame, &paint, &stroke, Transform::identity(), None);
            }
            pb.move_to(x + p(6.0), y + p(7.5));
            pb.line_to(x + p(9.5), y + p(10.0));
            pb.line_to(x + p(6.0), y + p(12.5));
            pb.move_to(x + p(11.0), y + p(13.0));
            pb.line_to(x + p(14.5), y + p(13.0));
        }
        Action::Audio => {
            pb.move_to(x + p(4.0), y + p(8.0));
            pb.line_to(x + p(7.0), y + p(8.0));
            pb.line_to(x + p(10.5), y + p(4.5));
            pb.line_to(x + p(10.5), y + p(15.5));
            pb.line_to(x + p(7.0), y + p(12.0));
            pb.line_to(x + p(4.0), y + p(12.0));
            pb.close();
            pb.move_to(x + p(13.5), y + p(7.5));
            pb.cubic_to(
                x + p(15.5),
                y + p(9.0),
                x + p(15.5),
                y + p(11.0),
                x + p(13.5),
                y + p(12.5),
            );
        }
        Action::Usb => {
            pb.move_to(x + p(10.0), y + p(16.5));
            pb.line_to(x + p(10.0), y + p(5.0));
            pb.move_to(x + p(7.5), y + p(7.5));
            pb.line_to(x + p(10.0), y + p(4.0));
            pb.line_to(x + p(12.5), y + p(7.5));
            pb.move_to(x + p(6.0), y + p(10.0));
            pb.line_to(x + p(10.0), y + p(10.0));
            pb.move_to(x + p(6.0), y + p(10.0));
            pb.line_to(x + p(6.0), y + p(13.0));
            pb.move_to(x + p(14.0), y + p(12.0));
            pb.line_to(x + p(10.0), y + p(12.0));
        }
        Action::Info => {
            if let Some(circle) = rounded_rect_path(x + p(2.5), y + p(2.5), p(15.0), p(15.0), p(7.5))
            {
                pixmap.stroke_path(&circle, &paint, &stroke, Transform::identity(), None);
            }
            pb.move_to(x + p(10.0), y + p(6.0));
            pb.line_to(x + p(10.0), y + p(7.0));
            pb.move_to(x + p(10.0), y + p(9.0));
            pb.line_to(x + p(10.0), y + p(14.0));
        }
        Action::Stop => {
            if let Some(sq) = rounded_rect_path(x + p(5.5), y + p(5.5), p(9.0), p(9.0), p(1.5)) {
                pixmap.fill_path(
                    &sq,
                    &paint,
                    FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
        }
    }

    if let Some(path) = pb.finish() {
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

/// Disclosure chevron as a stroked path.
pub fn draw_chevron(pixmap: &mut Pixmap, x: f32, center_y: f32, size: f32, expanded: bool, color: Rgba) {
    let arm = size * 0.5;
    let mut pb = PathBuilder::new();
    if expanded {
        pb.move_to(x + arm, center_y - arm);
        pb.line_to(x, center_y);
        pb.line_to(x + arm, center_y + arm);
    } else {
        pb.move_to(x, center_y - arm);
        pb.line_to(x + arm, center_y);
        pb.line_to(x, center_y + arm);
    }
    let Some(path) = pb.finish() else { return };
    let mut stroke = Stroke::default();
    stroke.width = (size * 0.16).max(1.0);
    stroke.line_cap = tiny_skia::LineCap::Round;
    stroke.line_join = tiny_skia::LineJoin::Round;
    pixmap.stroke_path(&path, &paint_of(color), &stroke, Transform::identity(), None);
}

/// The rendered tab, as premultiplied BGRA suitable for `wl_shm` directly.
pub fn tab_pixmap(spec: &ChromeSpec, fonts: &TextRenderer, width: u32, height: u32) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(width.max(1), height.max(1))?;
    let scale = spec.scale;
    let geo = TabGeometry {
        x: 8.0 * scale,
        y: 1.0 * scale,
        width: (width as f32) - 16.0 * scale,
        height: (height as f32) - 2.0 * scale,
        radius: 6.0 * scale,
        bar: 3.0 * scale,
        hairline: 1.0 * scale,
    };
    draw_tab_frame(&mut pixmap, &geo, spec.theme.plate, spec.accent, 0.45);
    let _ = fonts;
    Some(pixmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_rect_clamps_radius_and_builds_a_closed_path() {
        let p = rounded_rect_path(0.0, 0.0, 20.0, 10.0, 50.0).expect("path");
        // Radius is clamped to half the shorter side, so the path stays valid.
        assert!(p.bounds().width() <= 20.001);
        assert!(p.bounds().height() <= 10.001);
    }

    #[test]
    fn zero_sized_rect_yields_no_path() {
        assert!(rounded_rect_path(0.0, 0.0, 0.0, 10.0, 2.0).is_none());
        assert!(rounded_rect_path(0.0, 0.0, 10.0, 0.0, 2.0).is_none());
    }

    /// A stroke is centred on its path, so insetting by half the stroke width
    /// keeps the border inside the card and concentric with it. This is the
    /// property the hand-rolled version could not hold.
    #[test]
    fn border_stays_within_the_card() {
        let mut pm = Pixmap::new(60, 40).unwrap();
        let geo = TabGeometry {
            x: 5.0,
            y: 5.0,
            width: 50.0,
            height: 30.0,
            radius: 6.0,
            bar: 3.0,
            hairline: 1.0,
        };
        draw_tab_frame(&mut pm, &geo, Rgba::rgb(0x25, 0x27, 0x2b), Rgba::rgb(0xff, 0xb3, 0x47), 0.45);
        // Nothing is painted outside the card's bounding box.
        for y in 0..40u32 {
            for x in 0..60u32 {
                let inside = (5.0..55.0).contains(&(x as f32)) && (5.0..35.0).contains(&(y as f32));
                if !inside {
                    let px = pm.pixel(x, y).unwrap();
                    assert_eq!(px.alpha(), 0, "painted outside the card at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn pixmap_output_is_premultiplied() {
        // tiny_skia stores premultiplied pixels, which is exactly what wl_shm
        // expects. Painting a translucent white must not leave full-brightness
        // channels behind.
        let mut pm = Pixmap::new(4, 4).unwrap();
        let path = rounded_rect_path(0.0, 0.0, 4.0, 4.0, 0.0).unwrap();
        pm.fill_path(
            &path,
            &paint_of(Rgba::rgba(255, 255, 255, 128)),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
        let px = pm.pixel(1, 1).unwrap();
        assert!(px.red() < 200, "expected premultiplied, got {}", px.red());
        assert_eq!(px.alpha(), 128);
    }

    #[test]
    fn accent_bar_is_confined_to_the_left_edge() {
        let mut pm = Pixmap::new(60, 40).unwrap();
        let accent = Rgba::rgb(0xff, 0xb3, 0x47);
        let geo = TabGeometry {
            x: 5.0,
            y: 5.0,
            width: 50.0,
            height: 30.0,
            radius: 6.0,
            bar: 3.0,
            hairline: 1.0,
        };
        draw_tab_frame(&mut pm, &geo, Rgba::rgb(0x25, 0x27, 0x2b), accent, 0.45);
        // Well right of the bar and inside the card, the fill dominates.
        let px = pm.pixel(30, 20).unwrap();
        assert!(
            px.red() < 0x80,
            "accent leaked into the card body: {}",
            px.red()
        );
    }
}
