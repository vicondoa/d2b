//! The identity tab, rendered end to end with vector paths and outline text.
//!
//! This is the path the proxy uses. It exists because hand-written pixel loops
//! proved to be a source of defects rather than a saving, and because measuring
//! showed the vector stack is also far cheaper: a parsed face plus tiny-skia
//! costs roughly 0.9 MB resident, against about 19.5 MB for a bitmap
//! rasterizer's glyph cache on the same face.

use tiny_skia::Pixmap;

use crate::{
    color::{contrast_ratio, enforce_contrast, Rgba, CONTRAST_TEXT_AA},
    geom::{ChromeOutcome, LayoutInput, Size, TAB_INSET},
    skia::{self, TabGeometry},
    variant::{Action, ChromeSpec},
    vectext::VectorFont,
};

/// Layout constants, in logical px, shared by drawing and hit-testing.
pub const SIDE_PAD: f32 = 8.0;
pub const LEFT_FURNITURE: f32 = 4.0;
pub const CHEVRON_WIDTH: f32 = 9.0;
pub const CHEVRON_GAP: f32 = 5.0;
pub const ICON_BOX: f32 = 18.0;
pub const ICON_GAP: f32 = 4.0;
pub const SEP_GAP: f32 = 6.0;
pub const BAR_WIDTH: f32 = 3.0;
pub const HAIRLINE: f32 = 1.0;
pub const RADIUS: f32 = 6.0;

/// Where the tab and its parts sit, in physical px.
#[derive(Debug, Clone, Copy)]
pub struct TabLayout {
    pub band_height: u32,
    pub tab_x: f32,
    pub tab_y: f32,
    pub tab_width: f32,
    pub tab_height: f32,
    /// Width of the identity portion, before any action strip.
    pub identity_width: f32,
    pub scale: f32,
}

impl TabLayout {
    /// The pointer region, as `(x, y, w, h)` in physical px.
    pub fn input_region(&self) -> (i32, i32, i32, i32) {
        (
            self.tab_x as i32,
            self.tab_y as i32,
            self.tab_width.ceil() as i32,
            self.tab_height.ceil() as i32,
        )
    }

    /// Hit-test a pointer position: `None` outside, `Some(None)` on identity,
    /// `Some(Some(index))` on an action icon.
    #[allow(clippy::option_option)]
    pub fn hit(&self, x: f64, y: f64, actions: usize, expanded: bool) -> Option<Option<usize>> {
        let (x, y) = (x as f32, y as f32);
        if y < self.tab_y || y >= self.tab_y + self.tab_height {
            return None;
        }
        if x < self.tab_x || x >= self.tab_x + self.tab_width {
            return None;
        }
        let identity_end = self.tab_x + self.identity_width;
        if x < identity_end {
            return Some(None);
        }
        if !expanded || actions == 0 {
            return Some(None);
        }
        let s = self.scale;
        let first = identity_end + SEP_GAP * 2.0 * s + s;
        for i in 0..actions {
            let ix = first + i as f32 * (ICON_BOX + ICON_GAP) * s;
            if x >= ix - ICON_GAP * s / 2.0 && x < ix + (ICON_BOX + ICON_GAP / 2.0) * s {
                return Some(Some(i));
            }
        }
        Some(None)
    }
}

/// Resolve the tab's layout for a window of `content_width` physical px.
pub fn layout(spec: &ChromeSpec, font: &VectorFont<'_>, content_width: u32) -> Option<TabLayout> {
    let s = spec.scale;
    let px = spec.font_px * s;
    let tracking = spec.font_px * spec.tracking_em * s;

    let label_w = font.measure(&spec.label, px, tracking);
    let identity_width = (LEFT_FURNITURE + SIDE_PAD) * s
        + label_w
        + (CHEVRON_GAP + CHEVRON_WIDTH + SIDE_PAD) * s
        + HAIRLINE * s;

    let actions = if spec.expanded { spec.actions.len() } else { 0 };
    let actions_width = if actions == 0 {
        0.0
    } else {
        SEP_GAP * 2.0 * s
            + s
            + ICON_BOX * actions as f32 * s
            + ICON_GAP * (actions.saturating_sub(1)) as f32 * s
            + SIDE_PAD * s
    };

    // The band still comes from the shared geometry rules, so the accessibility
    // floors and the fail-closed contract continue to apply.
    let outcome = crate::geom::resolve(LayoutInput {
        content: Size::new(content_width, 1),
        button_width: identity_width.ceil() as u32,
        label_block_height: (px * 1.3).ceil() as u32,
        side_pad: (SIDE_PAD * s) as u32,
        vertical_pad: (TAB_INSET as f32 * s) as u32,
        accent_rule: 0,
        identity_verified: spec.identity_verified,
        ..Default::default()
    });
    let ChromeOutcome::Decorate(l) = outcome else {
        return None;
    };

    Some(TabLayout {
        band_height: l.band.height,
        tab_x: l.button.x as f32,
        tab_y: l.button.y as f32,
        tab_width: identity_width + actions_width,
        tab_height: l.button.height as f32,
        identity_width,
        scale: s,
    })
}

/// Render the band. Returns premultiplied BGRA bytes ready for `wl_shm`, and
/// the layout used, so hit-testing cannot disagree with what was drawn.
pub fn render_band(
    spec: &ChromeSpec,
    font: &VectorFont<'_>,
    width: u32,
) -> Option<(Vec<u8>, TabLayout)> {
    let l = layout(spec, font, width)?;
    let mut pm = Pixmap::new(width.max(1), l.band_height.max(1))?;
    let s = l.scale;

    let geo = TabGeometry {
        x: l.tab_x,
        y: l.tab_y,
        width: l.tab_width,
        height: l.tab_height,
        radius: RADIUS * s,
        bar: BAR_WIDTH * s,
        hairline: HAIRLINE * s,
    };

    let mut fill = spec.theme.plate;
    if spec.state.pressed {
        fill = fill.mix(Rgba::BLACK, 0.30);
    } else if spec.state.menu_open || spec.expanded {
        fill = fill.mix(Rgba::WHITE, 0.10);
    } else if spec.state.hover {
        fill = fill.mix(Rgba::WHITE, 0.06);
    }

    skia::draw_tab_frame(&mut pm, &geo, fill, spec.accent, 0.45);

    let fg = enforce_contrast(spec.theme.foreground, fill, CONTRAST_TEXT_AA)
        .unwrap_or(spec.theme.foreground);
    let px = spec.font_px * s;
    let tracking = spec.font_px * spec.tracking_em * s;

    // Centre on cap height so the label sits optically centred, not centred on
    // the full ascender-to-descender box.
    let cap = font.cap_height(px);
    let baseline = l.tab_y + (l.tab_height + cap) / 2.0;
    let text_x = l.tab_x + (LEFT_FURNITURE + SIDE_PAD) * s;
    let advance = font.draw(&mut pm, &spec.label, px, tracking, text_x, baseline, fg);

    skia::draw_chevron(
        &mut pm,
        text_x + advance + CHEVRON_GAP * s,
        l.tab_y + l.tab_height / 2.0,
        CHEVRON_WIDTH * s,
        spec.expanded,
        fg.with_alpha(0xd0),
    );

    if spec.expanded {
        let first = l.tab_x + l.identity_width + SEP_GAP * 2.0 * s + s;
        // Separator, with equal space either side.
        let sep_x = l.tab_x + l.identity_width + SEP_GAP * s;
        if let Some(sep) = skia::rounded_rect_path(
            sep_x,
            l.tab_y + 4.0 * s,
            (1.0 * s).max(1.0),
            l.tab_height - 8.0 * s,
            0.0,
        ) {
            let mut paint = tiny_skia::Paint::default();
            let c = fg.with_alpha(0x50);
            paint.set_color_rgba8(c.r, c.g, c.b, c.a);
            pm.fill_path(
                &sep,
                &paint,
                tiny_skia::FillRule::Winding,
                tiny_skia::Transform::identity(),
                None,
            );
        }
        for (i, action) in spec.actions.iter().enumerate() {
            let ix = first + i as f32 * (ICON_BOX + ICON_GAP) * s;
            let iy = l.tab_y + (l.tab_height - ICON_BOX * s) / 2.0;
            let active = spec.active_actions & (1 << i) != 0;
            if active
                && let Some(pill) = skia::rounded_rect_path(
                    ix - 3.0 * s,
                    iy - 2.0 * s,
                    (ICON_BOX + 6.0) * s,
                    (ICON_BOX + 4.0) * s,
                    4.0 * s,
                )
            {
                let mut paint = tiny_skia::Paint::default();
                paint.set_color_rgba8(spec.accent.r, spec.accent.g, spec.accent.b, 255);
                paint.anti_alias = true;
                pm.fill_path(
                    &pill,
                    &paint,
                    tiny_skia::FillRule::Winding,
                    tiny_skia::Transform::identity(),
                    None,
                );
            }
            let icon_fg = if active { fill } else { fg };
            skia::draw_action_icon(&mut pm, *action, ix, iy, ICON_BOX * s, icon_fg);
        }
    }

    // tiny_skia stores RGBA premultiplied; wl_shm wants BGRA premultiplied.
    let mut out = pm.take();
    for px in out.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    Some((out, l))
}

/// Contrast of the label against its own background, for acceptance checks.
pub fn label_contrast(spec: &ChromeSpec) -> f64 {
    contrast_ratio(spec.theme.foreground, spec.theme.plate)
}

/// Names for dispatch and logging.
pub fn action_name(index: usize) -> Option<&'static str> {
    Action::DEFAULTS.get(index).map(|a| match a {
        Action::Terminal => "terminal",
        Action::Audio => "audio",
        Action::Usb => "usb",
        Action::Info => "info",
        Action::Stop => "stop",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{variant::Candidate, PROTOTYPE_FONT};

    fn font() -> VectorFont<'static> {
        VectorFont::from_bytes(PROTOTYPE_FONT).unwrap()
    }

    fn spec() -> ChromeSpec {
        ChromeSpec::new(Candidate::Tab, "Work", Rgba::rgb(0xff, 0xb3, 0x47))
    }

    #[test]
    fn band_renders_and_matches_its_layout() {
        let f = font();
        let (bytes, l) = render_band(&spec(), &f, 1280).expect("renders");
        assert_eq!(bytes.len(), 1280 * l.band_height as usize * 4);
        assert!(l.band_height >= crate::geom::MIN_BAND_HEIGHT);
    }

    #[test]
    fn expanding_widens_the_tab_and_the_region() {
        let f = font();
        let (_, narrow) = render_band(&spec(), &f, 1280).unwrap();
        let mut e = spec();
        e.expanded = true;
        let (_, wide) = render_band(&e, &f, 1280).unwrap();
        assert!(
            wide.tab_width > narrow.tab_width,
            "{} should exceed {}",
            wide.tab_width,
            narrow.tab_width
        );
        assert!(wide.input_region().2 > narrow.input_region().2);
    }

    #[test]
    fn identity_hit_is_distinct_from_action_hits() {
        let f = font();
        let mut e = spec();
        e.expanded = true;
        let (_, l) = render_band(&e, &f, 1280).unwrap();
        let mid_y = f64::from(l.tab_y + l.tab_height / 2.0);

        // Inside the identity portion.
        assert_eq!(
            l.hit(f64::from(l.tab_x + 10.0), mid_y, 5, true),
            Some(None)
        );
        // Each icon resolves to its own index, in order.
        let mut seen = Vec::new();
        let mut x = f64::from(l.tab_x + l.identity_width) + 8.0;
        while x < f64::from(l.tab_x + l.tab_width) {
            if let Some(Some(i)) = l.hit(x, mid_y, 5, true) {
                if seen.last() != Some(&i) {
                    seen.push(i);
                }
            }
            x += 1.0;
        }
        assert_eq!(seen, vec![0, 1, 2, 3, 4], "icons must resolve in order");
    }

    #[test]
    fn pointer_outside_the_tab_hits_nothing() {
        let f = font();
        let (_, l) = render_band(&spec(), &f, 1280).unwrap();
        let mid_y = f64::from(l.tab_y + l.tab_height / 2.0);
        assert_eq!(l.hit(f64::from(l.tab_x) - 4.0, mid_y, 5, false), None);
        assert_eq!(
            l.hit(f64::from(l.tab_x + l.tab_width) + 4.0, mid_y, 5, false),
            None
        );
        assert_eq!(l.hit(f64::from(l.tab_x + 10.0), 0.0, 5, false), None);
    }

    #[test]
    fn input_region_never_reaches_the_window_edges() {
        let f = font();
        let (_, l) = render_band(&spec(), &f, 1280).unwrap();
        let (x, y, w, h) = l.input_region();
        assert!(x > 0, "region touches the left edge");
        assert!(x + w < 1280, "region touches the right edge");
        assert!(y >= 0);
        assert!(y + h <= l.band_height as i32, "region leaves the band");
    }

    #[test]
    fn output_is_premultiplied_bgra() {
        let f = font();
        let (bytes, _) = render_band(&spec(), &f, 200).unwrap();
        // Every pixel must satisfy the premultiplied invariant: no colour
        // channel may exceed its alpha.
        for px in bytes.chunks_exact(4) {
            let a = px[3];
            assert!(px[0] <= a && px[1] <= a && px[2] <= a, "not premultiplied: {px:?}");
        }
    }

    #[test]
    fn unverified_identity_refuses_to_render() {
        let f = font();
        let mut s = spec();
        s.identity_verified = false;
        assert!(render_band(&s, &f, 1280).is_none());
    }
}
