//! The identity tab, rendered end to end with vector paths and outline text.
//!
//! This is the path the proxy uses. It exists because hand-written pixel loops
//! proved to be a source of defects rather than a saving, and because measuring
//! showed the vector stack is also far cheaper: a parsed face plus tiny-skia
//! costs roughly 0.9 MB resident, against about 19.5 MB for a bitmap
//! rasterizer's glyph cache on the same face.
//!
//! Layout comes from [`crate::parts`]. Drawing and hit-testing both walk the
//! same measured list, so what the user clicks is what they aimed at.

use tiny_skia::Pixmap;

use crate::{
    color::{contrast_ratio, enforce_contrast, Rgba, CONTRAST_TEXT_AA},
    geom::{ChromeOutcome, LayoutInput, Size, TAB_INSET},
    parts::{HitKind, Metrics, Part, Parts, PartsConfig},
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
pub const LABEL_GAP: f32 = 5.0;

/// Scale the logical constants into the physical metrics the parts model uses.
fn metrics(spec: &ChromeSpec, cfg: &PartsConfig) -> Metrics {
    let s = spec.scale;
    Metrics {
        scale: s,
        font_px: spec.font_px * s,
        tracking: spec.font_px * spec.tracking_em * s,
        left_furniture: (LEFT_FURNITURE + BAR_WIDTH) * s,
        side_pad: SIDE_PAD * s,
        chevron_width: CHEVRON_WIDTH * s,
        chevron_gap: CHEVRON_GAP * s,
        icon_box: ICON_BOX * s,
        icon_gap: ICON_GAP * s,
        sep_gap: SEP_GAP * s,
        sep_width: HAIRLINE * s,
        action_labels: !cfg.compact_actions,
        label_gap: LABEL_GAP * s,
    }
}

/// Where the tab and its parts sit, in physical px.
#[derive(Debug, Clone)]
pub struct TabLayout {
    pub band_height: u32,
    pub tab_x: f32,
    pub tab_y: f32,
    pub tab_width: f32,
    pub tab_height: f32,
    pub scale: f32,
    /// The measured parts, tab-relative. Drawing and hit-testing share this.
    pub parts: Parts,
}

impl TabLayout {
    /// The pointer region, as `(x, y, w, h)` in physical px.
    ///
    /// Derived from the same measurement that drew the tab, so the region can
    /// no longer end up beside the visible tab.
    pub fn input_region(&self) -> (i32, i32, i32, i32) {
        (
            self.tab_x as i32,
            self.tab_y as i32,
            self.tab_width.ceil() as i32,
            self.tab_height.ceil() as i32,
        )
    }

    /// Resolve a pointer position to the meaning of a press there.
    ///
    /// The accepted bounds are exactly [`Self::input_region`], not the
    /// unrounded tab rect. The region is ceiled to whole pixels when it is
    /// handed to the compositor, so testing against the unrounded rect would
    /// leave a sub-pixel sliver that receives events but resolves to nothing --
    /// a press the user can land on that silently does nothing.
    pub fn hit_kind(&self, x: f64, y: f64) -> Option<HitKind> {
        let (rx, ry, rw, rh) = self.input_region();
        let (x, y) = (x as f32, y as f32);
        if y < ry as f32 || y >= (ry + rh) as f32 {
            return None;
        }
        if x < rx as f32 || x >= (rx + rw) as f32 {
            return None;
        }
        // Clamp into the measured tab so the ceiled edge resolves to the part
        // that was drawn there.
        let local = (x - self.tab_x).clamp(0.0, self.parts.width);
        self.parts.hit(local).map(|p| p.part.hit_kind())
    }

    /// Hit-test in the shape the proxy consumes: `None` outside the tab,
    /// `Some(None)` for expand/collapse, `Some(Some(action))` for an action.
    #[allow(clippy::option_option)]
    pub fn hit(&self, x: f64, y: f64) -> Option<Option<Action>> {
        match self.hit_kind(x, y) {
            None => None,
            Some(HitKind::Action(a)) => Some(Some(a)),
            Some(_) => Some(None),
        }
    }
}

/// Resolve the tab's layout for a window of `content_width` physical px.
pub fn layout(
    spec: &ChromeSpec,
    cfg: &PartsConfig,
    font: &VectorFont<'_>,
    content_width: u32,
) -> Option<TabLayout> {
    let m = metrics(spec, cfg);
    let row = cfg.row(spec.expanded);
    let parts = Parts::layout(row, &m, font, &spec.label);

    // The band still comes from the shared geometry rules, so the accessibility
    // floors and the fail-closed contract continue to apply.
    let outcome = crate::geom::resolve(LayoutInput {
        content: Size::new(content_width, 1),
        button_width: parts.width.ceil() as u32,
        label_block_height: (m.font_px * 1.3).ceil() as u32,
        side_pad: m.side_pad as u32,
        vertical_pad: (TAB_INSET as f32 * spec.scale) as u32,
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
        tab_width: parts.width,
        tab_height: l.button.height as f32,
        scale: spec.scale,
        parts,
    })
}

/// Render the band. Returns premultiplied BGRA bytes ready for `wl_shm`, and
/// the layout used, so hit-testing cannot disagree with what was drawn.
pub fn render_band(
    spec: &ChromeSpec,
    cfg: &PartsConfig,
    font: &VectorFont<'_>,
    width: u32,
) -> Option<(Vec<u8>, TabLayout)> {
    let l = layout(spec, cfg, font, width)?;
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
    let m = metrics(spec, cfg);

    // Centre on cap height so the label sits optically centred, not centred on
    // the full ascender-to-descender box.
    let cap = font.cap_height(m.font_px);
    let baseline = l.tab_y + (l.tab_height + cap) / 2.0;
    let mid_y = l.tab_y + l.tab_height / 2.0;

    for placed in &l.parts.placed {
        let x = l.tab_x + placed.x;
        match &placed.part {
            Part::Identity => {
                // Centre within the (possibly widened) box, so a very short
                // workload name stays optically centred on its own target.
                let cx = x + (placed.width - placed.text_advance) / 2.0;
                font.draw(&mut pm, &spec.label, m.font_px, m.tracking, cx, baseline, fg);
            }
            Part::Status => {
                if let Some(status) = &spec.status {
                    font.draw(
                        &mut pm,
                        status,
                        m.font_px * 0.85,
                        m.tracking,
                        x,
                        baseline,
                        spec.theme.foreground_dim,
                    );
                }
            }
            Part::Chevron => {
                // The part box was widened to the target-size floor, but the
                // glyph must keep its designed size: growing the hit box is an
                // accessibility fix, growing the mark is a drawing bug.
                skia::draw_chevron(
                    &mut pm,
                    x + (placed.width - m.chevron_width) / 2.0,
                    mid_y,
                    m.chevron_width,
                    spec.expanded,
                    fg.with_alpha(0xd0),
                );
            }
            Part::Separator => {
                if let Some(sep) = skia::rounded_rect_path(
                    x,
                    l.tab_y + 4.0 * s,
                    placed.width.max(1.0),
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
            }
            Part::Action(action) => {
                let idx = spec.actions.iter().position(|a| a == action);
                let active = idx.is_some_and(|i| spec.active_actions & (1 << i) != 0);
                let icon = m.icon_box;
                let iy = l.tab_y + (l.tab_height - icon) / 2.0;
                // The part box may be wider than its contents, because every
                // interactive part is widened to the target-size floor. Centre
                // the contents in the box so the icon does not drift left of
                // the area that responds to it.
                let content = if m.action_labels {
                    icon + m.label_gap + placed.text_advance
                } else {
                    icon
                };
                let cx = x + (placed.width - content) / 2.0;

                if active
                    && let Some(pill) = skia::rounded_rect_path(
                        x,
                        l.tab_y + 2.0 * s,
                        placed.width,
                        l.tab_height - 4.0 * s,
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
                skia::draw_action_icon(&mut pm, *action, cx, iy, icon, icon_fg);
                if m.action_labels {
                    font.draw(
                        &mut pm,
                        action.label(),
                        m.font_px,
                        m.tracking,
                        cx + icon + m.label_gap,
                        baseline,
                        icon_fg,
                    );
                }
            }
            Part::Spacer(_) => {}
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
    Action::DEFAULTS.get(index).map(Action::name)
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

    fn cfg() -> PartsConfig {
        PartsConfig::default()
    }

    #[test]
    fn band_renders_and_matches_its_layout() {
        let f = font();
        let (bytes, l) = render_band(&spec(), &cfg(), &f, 1280).expect("renders");
        assert_eq!(bytes.len(), 1280 * l.band_height as usize * 4);
        assert!(l.band_height >= crate::geom::MIN_BAND_HEIGHT);
    }

    #[test]
    fn expanding_widens_the_tab_and_the_region() {
        let f = font();
        let (_, narrow) = render_band(&spec(), &cfg(), &f, 1280).unwrap();
        let mut e = spec();
        e.expanded = true;
        let (_, wide) = render_band(&e, &cfg(), &f, 1280).unwrap();
        assert!(
            wide.tab_width > narrow.tab_width,
            "{} should exceed {}",
            wide.tab_width,
            narrow.tab_width
        );
        assert!(wide.input_region().2 > narrow.input_region().2);
    }

    #[test]
    fn every_action_resolves_in_order_across_the_tab() {
        let f = font();
        let mut e = spec();
        e.expanded = true;
        let (_, l) = render_band(&e, &cfg(), &f, 1280).unwrap();
        let mid_y = f64::from(l.tab_y + l.tab_height / 2.0);

        let mut seen: Vec<Action> = Vec::new();
        let mut x = f64::from(l.tab_x);
        while x < f64::from(l.tab_x + l.tab_width) {
            if let Some(Some(a)) = l.hit(x, mid_y) {
                if seen.last() != Some(&a) {
                    seen.push(a);
                }
            }
            x += 0.5;
        }
        assert_eq!(seen, Action::DEFAULTS.to_vec(), "icons must resolve in order");
    }

    #[test]
    fn identity_and_chevron_both_toggle() {
        let f = font();
        let (_, l) = render_band(&spec(), &cfg(), &f, 1280).unwrap();
        let mid_y = f64::from(l.tab_y + l.tab_height / 2.0);
        let identity = l.parts.find("identity").unwrap();
        let chevron = l.parts.find("chevron").unwrap();
        for p in [identity, chevron] {
            let x = f64::from(l.tab_x + p.x + p.width / 2.0);
            assert_eq!(l.hit_kind(x, mid_y), Some(HitKind::Toggle));
        }
    }

    #[test]
    fn no_press_inside_the_tab_is_swallowed() {
        // The operator-reported defect: presses that landed in padding did
        // nothing at all.
        let f = font();
        let mut e = spec();
        e.expanded = true;
        let (_, l) = render_band(&e, &cfg(), &f, 1280).unwrap();
        let mid_y = f64::from(l.tab_y + l.tab_height / 2.0);
        let mut x = f64::from(l.tab_x);
        while x < f64::from(l.tab_x + l.tab_width) {
            assert!(l.hit_kind(x, mid_y).is_some(), "dead press at {x}");
            x += 0.5;
        }
    }

    #[test]
    fn pointer_outside_the_tab_hits_nothing() {
        let f = font();
        let (_, l) = render_band(&spec(), &cfg(), &f, 1280).unwrap();
        let mid_y = f64::from(l.tab_y + l.tab_height / 2.0);
        assert_eq!(l.hit(f64::from(l.tab_x) - 4.0, mid_y), None);
        assert_eq!(l.hit(f64::from(l.tab_x + l.tab_width) + 4.0, mid_y), None);
        assert_eq!(l.hit(f64::from(l.tab_x + 10.0), 0.0), None);
    }

    #[test]
    fn input_region_never_reaches_the_window_edges() {
        let f = font();
        let (_, l) = render_band(&spec(), &cfg(), &f, 1280).unwrap();
        let (x, y, w, h) = l.input_region();
        assert!(x > 0, "region touches the left edge");
        assert!(x + w < 1280, "region touches the right edge");
        assert!(y >= 0);
        assert!(y + h <= l.band_height as i32, "region leaves the band");
    }

    #[test]
    fn input_region_matches_the_drawn_tab() {
        let f = font();
        let mut e = spec();
        e.expanded = true;
        let (_, l) = render_band(&e, &cfg(), &f, 1280).unwrap();
        let (rx, _, rw, _) = l.input_region();
        assert!(f64::from(rx) <= f64::from(l.tab_x) + 1.0);
        assert!(
            f64::from(rx + rw) >= f64::from(l.tab_x + l.tab_width) - 1.0,
            "region {rw} narrower than the drawn tab {}",
            l.tab_width
        );
    }

    #[test]
    fn output_is_premultiplied_bgra() {
        let f = font();
        let (bytes, _) = render_band(&spec(), &cfg(), &f, 200).unwrap();
        // Every pixel must satisfy the premultiplied invariant: no colour
        // channel may exceed its alpha.
        for px in bytes.chunks_exact(4) {
            let a = px[3];
            assert!(
                px[0] <= a && px[1] <= a && px[2] <= a,
                "not premultiplied: {px:?}"
            );
        }
    }

    #[test]
    fn unverified_identity_refuses_to_render() {
        let f = font();
        let mut s = spec();
        s.identity_verified = false;
        assert!(render_band(&s, &cfg(), &f, 1280).is_none());
    }

    #[test]
    fn a_custom_row_renders_and_hits_in_its_configured_order() {
        // Customization has to survive the whole pipeline, not just the
        // layout unit test.
        let f = font();
        let custom = PartsConfig {
            collapsed: vec![Part::Identity, Part::Chevron],
            expanded: vec![
                Part::Identity,
                Part::Chevron,
                Part::Separator,
                Part::Action(Action::Stop),
                Part::Action(Action::Terminal),
            ],
            compact_actions: false,
        };
        custom.validate().unwrap();
        let mut e = spec();
        e.expanded = true;
        let (_, l) = render_band(&e, &custom, &f, 1280).unwrap();
        let mid_y = f64::from(l.tab_y + l.tab_height / 2.0);

        let stop = l.parts.find("stop-vm").unwrap();
        let term = l.parts.find("open-terminal").unwrap();
        assert!(stop.x < term.x, "configured order must be preserved");
        assert_eq!(
            l.hit(f64::from(l.tab_x + stop.x + stop.width / 2.0), mid_y),
            Some(Some(Action::Stop))
        );
        assert_eq!(
            l.hit(f64::from(l.tab_x + term.x + term.width / 2.0), mid_y),
            Some(Some(Action::Terminal))
        );
    }
}
