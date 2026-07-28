//! Rendering for the window identity tab and its deliberate controls.
//!
//! `Candidate::BandNeutral` is the design the UX panel converged on across five
//! rounds. The other arms are controls: they exist so the panel's arguments are
//! *visible* to the operator rather than asserted in prose.

use crate::{
    canvas::Canvas,
    color::{contrast_ratio, enforce_contrast, readable_on, Rgba, CONTRAST_TEXT_AA},
    geom::{BandPlacement, ChromeLayout, ChromeOutcome, LayoutInput, Size},
    text::{blend_glyph, TextRenderer},
};

/// What to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Candidate {
    /// A — the candidate. Painted neutral band, identity button, accent rule.
    BandNeutral,
    /// B — control. Same reserved geometry, band left transparent.
    BandTransparent,
    /// C — control. Accent-filled button with auto-contrast text, to show the
    /// thin 4.58:1 worst case in practice.
    AccentFill,
    /// D — control. Outside-geometry notch: no reservation, paints over niri's
    /// border, and is erased entirely by `clip-to-geometry true`.
    OutsideNotch,
}

impl Candidate {
    pub fn id(self) -> &'static str {
        match self {
            Self::BandNeutral => "A",
            Self::BandTransparent => "B",
            Self::AccentFill => "C",
            Self::OutsideNotch => "D",
        }
    }
}

/// Interaction and focus state. Identity presence never varies with these;
/// only emphasis does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VisualState {
    pub window_focused: bool,
    pub hover: bool,
    pub pressed: bool,
    pub menu_open: bool,
    pub keyboard_focus: bool,
}

impl VisualState {
    pub fn focused() -> Self {
        Self {
            window_focused: true,
            ..Default::default()
        }
    }
}

/// Neutral chrome tokens. Deliberately renderer-neutral so the same values can
/// drive proxy drawing and any toolkit path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub surface: Rgba,
    pub foreground: Rgba,
    pub outline: Rgba,
    pub focus: Rgba,
    pub radius: f64,
    pub accent_rule: u32,
    pub side_pad: u32,
    pub vertical_pad: u32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            surface: Rgba::rgb(0x1c, 0x1c, 0x22),
            foreground: Rgba::rgb(0xe8, 0xe8, 0xef),
            outline: Rgba::rgba(0xff, 0xff, 0xff, 0x24),
            focus: Rgba::rgb(0xff, 0xff, 0xff),
            radius: 6.0,
            accent_rule: 4,
            side_pad: 10,
            vertical_pad: 5,
        }
    }
}

/// Everything needed to draw one window's chrome.
#[derive(Debug, Clone)]
pub struct ChromeSpec {
    pub candidate: Candidate,
    /// Human display name, resolved and uniqueness-checked upstream.
    pub label: String,
    /// Composed security-capability token, e.g. `MIC MUTED · USB`.
    pub status: Option<String>,
    pub accent: Rgba,
    pub theme: Theme,
    pub state: VisualState,
    /// Logical to physical scale.
    pub scale: f32,
    /// Label size in logical px. 14 default, 12 floor.
    pub font_px: f32,
    /// Extra letter-spacing in em, for the WCAG 1.4.12 pass.
    pub tracking_em: f32,
    pub content_width: u32,
    pub content_height: u32,
    /// Whether host-verified identity is available.
    pub identity_verified: bool,
}

impl ChromeSpec {
    pub fn new(candidate: Candidate, label: impl Into<String>, accent: Rgba) -> Self {
        Self {
            candidate,
            label: label.into(),
            status: None,
            accent,
            theme: Theme::default(),
            state: VisualState::focused(),
            scale: 1.0,
            font_px: 14.0,
            tracking_em: 0.0,
            content_width: 880,
            content_height: 480,
            identity_verified: true,
        }
    }
}

/// A rendered result plus the facts the acceptance criteria are checked against.
pub struct Rendered {
    pub canvas: Canvas,
    pub layout: Option<ChromeLayout>,
    /// Measured contrast of label text against what sits directly behind it.
    pub label_contrast: f64,
    /// Whether chrome blocked the guest instead of decorating it.
    pub blocked: bool,
}

fn px(v: u32, scale: f32) -> u32 {
    ((v as f32) * scale).round() as u32
}

/// Resolve layout for a spec, measuring the label rather than guessing.
/// Returns the outcome and the measured button width.
pub fn resolve_for(spec: &ChromeSpec, fonts: &TextRenderer) -> (ChromeOutcome, u32) {
    let font_px = spec.font_px * spec.scale;
    let tracking = spec.font_px * spec.tracking_em * spec.scale;
    let metrics = fonts.measure(&spec.label, font_px, tracking);
    let line_h = (font_px * 1.3).ceil() as u32;
    let button_w = metrics.width + px(spec.theme.side_pad, spec.scale) * 2;
    let status_w = spec
        .status
        .as_deref()
        .map(|s| fonts.measure(s, font_px * 0.9, tracking).width + px(10, spec.scale) * 2)
        .unwrap_or(0);

    let input = LayoutInput {
        content: Size::new(
            px(spec.content_width, spec.scale),
            px(spec.content_height, spec.scale),
        ),
        placement: BandPlacement::Top,
        button_width: button_w,
        label_block_height: line_h,
        label_wrapped: false,
        status_width: status_w,
        side_pad: px(spec.theme.side_pad, spec.scale),
        vertical_pad: px(spec.theme.vertical_pad, spec.scale),
        accent_rule: px(spec.theme.accent_rule, spec.scale),
        identity_verified: spec.identity_verified,
    };
    (crate::geom::resolve(input), button_w)
}

/// Render chrome over a background that stands in for guest content.
pub fn render(spec: &ChromeSpec, fonts: &TextRenderer, background: Rgba) -> Rendered {
    if spec.candidate == Candidate::OutsideNotch && spec.identity_verified {
        return render_outside_notch(spec, fonts, background);
    }

    let (outcome, _) = resolve_for(spec, fonts);
    let layout = match outcome {
        ChromeOutcome::Decorate(l) => l,
        ChromeOutcome::FailClosed(_) => return render_blocked(spec, fonts),
    };

    let w = layout.outer.width as usize;
    let h = layout.outer.height as usize;
    let mut canvas = Canvas::new(w, h, Rgba::TRANSPARENT);

    let c = layout.content_rect();
    canvas.fill_rect(c.x, c.y, c.width, c.height, background);

    let scale = spec.scale;
    let band = layout.band;
    let button = layout.button;

    // Unfocused softens the surface but never the label contrast.
    let surface = if spec.state.window_focused {
        spec.theme.surface
    } else {
        spec.theme.surface.mix(background, 0.18)
    };

    let button_bg = match spec.candidate {
        Candidate::AccentFill => spec.accent,
        _ => {
            // The button must read as a control, not as a patch of the band.
            // Lift it off the band surface and outline it below.
            let mut s = surface.mix(Rgba::WHITE, 0.09);
            if spec.state.pressed {
                s = s.mix(Rgba::BLACK, 0.28);
            } else if spec.state.hover || spec.state.menu_open {
                s = s.mix(Rgba::WHITE, 0.10);
            }
            s
        }
    };

    if matches!(spec.candidate, Candidate::BandNeutral | Candidate::AccentFill) {
        canvas.fill_rect(band.x, band.y, band.width, band.height, surface);
        canvas.fill_rect(band.x, band.bottom() - 1, band.width, 1, spec.theme.outline);
    }
    // Control B paints no band: only the button below.

    canvas.fill_round_rect(
        button.x,
        button.y,
        button.width,
        button.height,
        spec.theme.radius * f64::from(scale),
        button_bg,
    );
    if spec.candidate != Candidate::AccentFill {
        canvas.stroke_rect(
            button.x,
            button.y,
            button.width,
            button.height,
            spec.theme.outline,
        );
    }

    // The identity colour is a rule under the button, not the text background.
    // Control C omits it because its fill carries the accent instead.
    let rule = px(spec.theme.accent_rule, scale).max(1);
    if spec.candidate != Candidate::AccentFill {
        canvas.fill_rect(
            button.x,
            button.bottom() - rule as i32,
            button.width,
            rule,
            spec.accent,
        );
    }

    let font_px = spec.font_px * scale;
    let tracking = spec.font_px * spec.tracking_em * scale;
    let desired = match spec.candidate {
        Candidate::AccentFill => readable_on(button_bg),
        _ => spec.theme.foreground,
    };
    let fg = enforce_contrast(desired, button_bg, CONTRAST_TEXT_AA).unwrap_or(desired);
    let label_contrast = contrast_ratio(fg, button_bg);

    let m = fonts.measure(&spec.label, font_px, tracking);
    let text_x = button.x + px(spec.theme.side_pad, scale) as i32;
    let face_free = button.height.saturating_sub(rule);
    let baseline = button.y + ((face_free + m.ascent) / 2) as i32;
    for g in fonts.layout(&spec.label, font_px, tracking, text_x, baseline) {
        blend_glyph(&mut canvas.pixels, w, h, &g, fg);
    }

    // Keyboard focus: light outer ring plus a dark inner ring reads on any fill.
    if spec.state.keyboard_focus {
        let r = button;
        canvas.stroke_rect(r.x - 3, r.y - 3, r.width + 6, r.height + 6, spec.theme.focus);
        canvas.stroke_rect(r.x - 2, r.y - 2, r.width + 4, r.height + 4, spec.theme.focus);
        canvas.stroke_rect(r.x - 1, r.y - 1, r.width + 2, r.height + 2, Rgba::BLACK);
    }

    if let (Some(status), Some(rect)) = (spec.status.as_deref(), layout.status) {
        let bg = surface.mix(Rgba::WHITE, 0.08);
        canvas.fill_round_rect(
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            spec.theme.radius * f64::from(scale),
            bg,
        );
        let sfg = enforce_contrast(spec.theme.foreground, bg, CONTRAST_TEXT_AA)
            .unwrap_or(spec.theme.foreground);
        let sp = font_px * 0.9;
        let sm = fonts.measure(status, sp, tracking);
        let sx = rect.x + ((rect.width.saturating_sub(sm.width)) / 2) as i32;
        let sbase = rect.y + ((rect.height + sm.ascent) / 2) as i32;
        for g in fonts.layout(status, sp, tracking, sx, sbase) {
            blend_glyph(&mut canvas.pixels, w, h, &g, sfg);
        }
    }

    Rendered {
        canvas,
        layout: Some(layout),
        label_contrast,
        blocked: false,
    }
}

/// Fail-closed: obscure the guest and state why. Never a bare window.
fn render_blocked(spec: &ChromeSpec, fonts: &TextRenderer) -> Rendered {
    let w = px(spec.content_width, spec.scale).max(1) as usize;
    let h = px(spec.content_height, spec.scale).max(1) as usize;
    let mut canvas = Canvas::new(w, h, spec.theme.surface);

    // A diagonal hatch: a pattern, not a hue, so the blocked state survives
    // grayscale and colour-vision deficiency.
    let hatch = spec.theme.foreground.with_alpha(0x18);
    let step = px(10, spec.scale).max(4) as usize;
    for y in 0..h {
        for x in 0..w {
            if (x + y) % step == 0 {
                canvas.blend(x as i32, y as i32, hatch);
            }
        }
    }

    let font_px = spec.font_px * spec.scale;
    let msg = "UNVERIFIED — content blocked";
    let m = fonts.measure(msg, font_px, 0.0);
    let x = ((w as i32) - (m.width as i32)) / 2;
    let fg = enforce_contrast(spec.theme.foreground, spec.theme.surface, CONTRAST_TEXT_AA)
        .unwrap_or(spec.theme.foreground);
    for g in fonts.layout(msg, font_px, 0.0, x, (h / 2) as i32) {
        blend_glyph(&mut canvas.pixels, w, h, &g, fg);
    }

    Rendered {
        canvas,
        layout: None,
        label_contrast: contrast_ratio(fg, spec.theme.surface),
        blocked: true,
    }
}

/// Control D: the outside-geometry notch the operator asked about. It costs no
/// layout space, paints over niri's border because client subsurfaces render
/// above it, and vanishes entirely under `clip-to-geometry true`.
fn render_outside_notch(spec: &ChromeSpec, fonts: &TextRenderer, background: Rgba) -> Rendered {
    let scale = spec.scale;
    let cw = px(spec.content_width, scale);
    let ch = px(spec.content_height, scale);
    let font_px = spec.font_px * scale;
    let m = fonts.measure(&spec.label, font_px, 0.0);
    let notch_h = px(22, scale);
    let notch_w = m.width + px(spec.theme.side_pad, scale) * 2;

    let w = cw as usize;
    let h = (ch + notch_h) as usize;
    let mut canvas = Canvas::new(w, h, Rgba::TRANSPARENT);
    canvas.fill_rect(0, notch_h as i32, cw, ch, background);

    canvas.fill_round_rect(
        px(12, scale) as i32,
        0,
        notch_w,
        notch_h,
        spec.theme.radius * f64::from(scale),
        spec.accent,
    );
    let fg = readable_on(spec.accent);
    let baseline = ((notch_h + m.ascent) / 2) as i32;
    for g in fonts.layout(
        &spec.label,
        font_px,
        0.0,
        px(12 + spec.theme.side_pad, scale) as i32,
        baseline,
    ) {
        blend_glyph(&mut canvas.pixels, w, h, &g, fg);
    }

    Rendered {
        canvas,
        layout: None,
        label_contrast: contrast_ratio(fg, spec.accent),
        blocked: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{geom::MIN_TARGET, PROTOTYPE_FONT};

    fn fonts() -> TextRenderer {
        TextRenderer::from_bytes(PROTOTYPE_FONT).unwrap()
    }

    fn spec(candidate: Candidate) -> ChromeSpec {
        ChromeSpec::new(candidate, "work", Rgba::rgb(0xff, 0xa5, 0x00))
    }

    const DARK: Rgba = Rgba::rgb(0x10, 0x10, 0x14);
    const LIGHT: Rgba = Rgba::rgb(0xf4, 0xf4, 0xf8);

    #[test]
    fn candidate_a_reserves_geometry_and_offsets_content() {
        let r = render(&spec(Candidate::BandNeutral), &fonts(), DARK);
        let l = r.layout.expect("candidate A decorates");
        assert!(l.content_origin.1 > 0, "content is pushed below the band");
        assert_eq!(l.outer.height, l.content.height + l.band.height);
        assert!(!r.blocked);
    }

    #[test]
    fn label_contrast_clears_the_body_text_floor_in_every_candidate() {
        let f = fonts();
        for candidate in [
            Candidate::BandNeutral,
            Candidate::BandTransparent,
            Candidate::AccentFill,
            Candidate::OutsideNotch,
        ] {
            for bg in [DARK, LIGHT] {
                let r = render(&spec(candidate), &f, bg);
                assert!(
                    r.label_contrast >= CONTRAST_TEXT_AA,
                    "{} on {bg:?} measured {:.2}",
                    candidate.id(),
                    r.label_contrast
                );
            }
        }
    }

    /// Control C exists to make this visible: an accent fill does pass, but
    /// with far less headroom than the neutral plate.
    #[test]
    fn accent_fill_has_thinner_margin_than_the_neutral_plate() {
        let f = fonts();
        // Near the analytic worst case for black/white selection.
        let worst = Rgba::rgb(47, 114, 222);

        let mut c = spec(Candidate::AccentFill);
        c.accent = worst;
        let fill = render(&c, &f, DARK);

        let mut n = spec(Candidate::BandNeutral);
        n.accent = worst;
        let neutral = render(&n, &f, DARK);

        assert!(fill.label_contrast >= CONTRAST_TEXT_AA);
        assert!(fill.label_contrast < 5.0, "fill margin is thin by construction");
        assert!(
            neutral.label_contrast > fill.label_contrast * 2.0,
            "neutral {:.2} should dominate fill {:.2}",
            neutral.label_contrast,
            fill.label_contrast
        );
    }

    #[test]
    fn identity_is_present_and_legible_in_every_visual_state() {
        let f = fonts();
        let states = [
            VisualState::focused(),
            VisualState::default(),
            VisualState {
                hover: true,
                ..VisualState::focused()
            },
            VisualState {
                pressed: true,
                ..VisualState::focused()
            },
            VisualState {
                menu_open: true,
                ..VisualState::focused()
            },
            VisualState {
                keyboard_focus: true,
                ..VisualState::focused()
            },
        ];
        for state in states {
            let mut s = spec(Candidate::BandNeutral);
            s.state = state;
            let r = render(&s, &f, DARK);
            assert!(
                r.label_contrast >= CONTRAST_TEXT_AA,
                "state {state:?} dropped contrast to {:.2}",
                r.label_contrast
            );
            assert!(r.layout.is_some(), "state {state:?} lost its chrome");
        }
    }

    #[test]
    fn input_region_holds_the_target_floor_at_fractional_scale() {
        let f = fonts();
        for scale in [1.0_f32, 1.25, 1.5, 2.0] {
            let mut s = spec(Candidate::BandNeutral);
            s.scale = scale;
            let (outcome, _) = resolve_for(&s, &f);
            let l = outcome.layout().expect("decorates at every scale");
            let r = l.input_region();
            assert!(r.width >= MIN_TARGET, "scale {scale}: width {}", r.width);
            assert!(r.height >= MIN_TARGET, "scale {scale}: height {}", r.height);
            assert!(!r.intersects(l.content_rect()));
            assert!(r.x > 0, "scale {scale}: must not touch the left edge");
        }
    }

    #[test]
    fn long_labels_widen_the_button_rather_than_clipping() {
        let f = fonts();
        let (_, short) = resolve_for(&spec(Candidate::BandNeutral), &f);
        let mut long = spec(Candidate::BandNeutral);
        long.label = "corp-workstation.work".to_owned();
        let (o, wide) = resolve_for(&long, &f);
        assert!(o.layout().is_some());
        assert!(
            wide > short,
            "long label must widen the face: {wide} vs {short}"
        );
    }

    #[test]
    fn wcag_text_spacing_widens_rather_than_overflows() {
        let f = fonts();
        let mut base = spec(Candidate::BandNeutral);
        base.label = "corp-workstation.work".to_owned();
        let (_, plain) = resolve_for(&base, &f);
        base.tracking_em = 0.12;
        let (_, spaced) = resolve_for(&base, &f);
        assert!(spaced > plain, "letter-spacing must grow the face");
    }

    #[test]
    fn two_hundred_percent_text_grows_the_band() {
        let f = fonts();
        let (base, _) = resolve_for(&spec(Candidate::BandNeutral), &f);
        let mut big = spec(Candidate::BandNeutral);
        big.font_px = 28.0;
        let (grown, _) = resolve_for(&big, &f);
        let b = base.layout().unwrap();
        let g = grown.layout().unwrap();
        assert!(
            g.band.height > b.band.height,
            "200% text must grow the band: {} vs {}",
            g.band.height,
            b.band.height
        );
        assert!(g.reflow.grew_band);
    }

    #[test]
    fn status_token_renders_without_touching_identity() {
        let mut s = spec(Candidate::BandNeutral);
        s.status = Some("MIC MUTED · USB".to_owned());
        let r = render(&s, &fonts(), DARK);
        let l = r.layout.unwrap();
        let st = l.status.expect("wide window keeps the token");
        assert!(!st.intersects(l.button));
        assert!(!st.intersects(l.content_rect()));
    }

    #[test]
    fn unverified_identity_blocks_the_guest_with_a_pattern() {
        let mut s = spec(Candidate::BandNeutral);
        s.identity_verified = false;
        let r = render(&s, &fonts(), DARK);
        assert!(r.blocked, "guest must be obscured, never shown bare");
        assert!(r.layout.is_none());
        assert!(r.label_contrast >= CONTRAST_TEXT_AA);
        // No realm accent anywhere in the blocked state.
        let accent = Rgba::rgb(0xff, 0xa5, 0x00);
        assert!(
            !r.canvas.pixels.iter().any(|p| *p == accent),
            "blocked state must not display the realm colour"
        );
    }

    #[test]
    fn control_d_costs_no_geometry_and_reserves_nothing() {
        let r = render(&spec(Candidate::OutsideNotch), &fonts(), DARK);
        assert!(
            r.layout.is_none(),
            "the notch reserves nothing, which is the point of the control"
        );
    }

    #[test]
    fn control_b_pays_the_same_cost_but_leaves_the_band_bare() {
        let f = fonts();
        let a = render(&spec(Candidate::BandNeutral), &f, DARK);
        let b = render(&spec(Candidate::BandTransparent), &f, DARK);
        let la = a.layout.unwrap();
        let lb = b.layout.unwrap();
        assert_eq!(la.outer, lb.outer, "B pays A's full geometry cost");

        // Sample the band far to the right of the identity button.
        let x = (lb.outer.width - 4) as usize;
        let y = (lb.band.y + 2) as usize;
        assert_eq!(b.canvas.get(x, y).a, 0, "B's band is transparent");
        assert!(a.canvas.get(x, y).a > 0, "A's band is painted");
    }

    #[test]
    fn grayscale_render_keeps_the_plate_achromatic() {
        let r = render(&spec(Candidate::BandNeutral), &fonts(), DARK);
        let gray = r.canvas.to_grayscale();
        let l = r.layout.unwrap();
        let p = gray.get((l.button.x + 2) as usize, (l.button.y + 2) as usize);
        assert_eq!(p.r, p.g);
        assert_eq!(p.g, p.b);
    }

    #[test]
    fn unfocused_softens_the_surface_without_weakening_identity() {
        let f = fonts();
        let focused = render(&spec(Candidate::BandNeutral), &f, DARK);
        let mut unfocused_spec = spec(Candidate::BandNeutral);
        unfocused_spec.state = VisualState::default();
        let unfocused = render(&unfocused_spec, &f, DARK);
        assert!(unfocused.label_contrast >= CONTRAST_TEXT_AA);
        assert!(
            (unfocused.label_contrast - focused.label_contrast).abs() < 2.0,
            "identity legibility must not collapse when unfocused"
        );
    }
}
