//! Rendering for the window identity tab and its deliberate controls.
//!
//! `Candidate::BandNeutral` is the design the UX panel converged on across five
//! rounds. The other arms are controls: they exist so the panel's arguments are
//! *visible* to the operator rather than asserted in prose.

use crate::{
    canvas::Canvas,
    color::{contrast_ratio, enforce_contrast, readable_on, Rgba, CONTRAST_TEXT_AA},
    geom::{BandPlacement, ChromeLayout, ChromeOutcome, LayoutInput, Rect, Size},
    text::{blend_glyph, TextRenderer},
};

/// What to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Candidate {
    /// The candidate. A compact tab styled like a `d2b-wlcontrol` realm card:
    /// thick accent left edge, thin neutral outline elsewhere, and a fully
    /// transparent surround so the reserved strip reads as empty space rather
    /// than a second titlebar.
    Tab,
    /// Control. The earlier full-width painted band, kept for comparison.
    BandNeutral,
    /// Control. Same reserved geometry, band left transparent.
    BandTransparent,
    /// Control. Accent-filled button with auto-contrast text, to show the
    /// thin 4.58:1 worst case in practice.
    AccentFill,
    /// Control. Outside-geometry notch: no reservation, paints over niri's
    /// border, and is erased entirely by `clip-to-geometry true`.
    OutsideNotch,
}

impl Candidate {
    pub fn id(self) -> &'static str {
        match self {
            Self::Tab => "T",
            Self::BandNeutral => "A",
            Self::BandTransparent => "B",
            Self::AccentFill => "C",
            Self::OutsideNotch => "D",
        }
    }

    /// Whether this arm paints the full-width band behind the tab.
    fn paints_band(self) -> bool {
        matches!(self, Self::BandNeutral | Self::AccentFill)
    }
}

/// One action offered when the tab is expanded. Icons are procedural so they
/// belong to the trusted renderer rather than a themeable icon set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Terminal,
    Audio,
    Usb,
    Info,
    Stop,
}

impl Action {
    pub const DEFAULTS: [Action; 5] = [
        Action::Terminal,
        Action::Audio,
        Action::Usb,
        Action::Info,
        Action::Stop,
    ];
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
    /// The band behind everything. Recessed relative to the plate.
    pub surface: Rgba,
    /// The identity button's plate. Must clear 3:1 against `surface` so the
    /// clickable target is delineated without relying on the accent.
    pub plate: Rgba,
    pub foreground: Rgba,
    /// Lower-emphasis foreground for the status token.
    pub foreground_dim: Rgba,
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
            surface: Rgba::rgb(0x0f, 0x11, 0x17),
            // The off-gray from d2b-wlcontrol's realm cards.
            plate: Rgba::rgb(0x25, 0x27, 0x2b),
            foreground: Rgba::rgb(0xf2, 0xf2, 0xf7),
            foreground_dim: Rgba::rgb(0xc2, 0xc2, 0xd0),
            outline: Rgba::rgba(0xff, 0xff, 0xff, 0x70),
            focus: Rgba::rgb(0xff, 0xff, 0xff),
            radius: 6.0,
            accent_rule: 4,
            side_pad: 8,
            vertical_pad: 2,
        }
    }
}

impl Theme {
    /// The boundary colour as actually composited over the band.
    pub fn composited_outline(&self) -> Rgba {
        self.outline.over(self.surface)
    }

    /// How strongly the identity target is delineated from the band, taking
    /// the better of its fill and its boundary. WCAG 1.4.11 wants 3:1.
    pub fn target_delineation(&self) -> f64 {
        contrast_ratio(self.plate, self.surface)
            .max(contrast_ratio(self.composited_outline(), self.surface))
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
    /// Interaction state of the status token, which activates the same menu
    /// and therefore needs the same visible states as the identity button.
    pub status_state: VisualState,
    /// Whether the tab is expanded to reveal its action icons.
    pub expanded: bool,
    /// Actions offered when expanded, in order.
    pub actions: Vec<Action>,
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
            status_state: VisualState::default(),
            expanded: false,
            actions: Action::DEFAULTS.to_vec(),
            scale: 1.0,
            font_px: 12.0,
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
    // Label, padding on both sides, plus room for the disclosure chevron.
    let button_w = metrics.width
        + px(spec.theme.side_pad, spec.scale) * 2
        + px(CHEVRON_WIDTH + 5, spec.scale);
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
            // The plate must read as a control. It clears 3:1 against the band
            // by construction, so the target is delineated without the accent.
            let mut s = if spec.state.window_focused {
                spec.theme.plate
            } else {
                spec.theme.plate.mix(surface, 0.25)
            };
            if spec.state.pressed {
                s = s.mix(Rgba::BLACK, 0.35);
            } else if spec.state.menu_open {
                s = s.mix(Rgba::WHITE, 0.18);
            } else if spec.state.hover {
                s = s.mix(Rgba::WHITE, 0.10);
            }
            s
        }
    };

    // Only the control arms paint the band. The candidate leaves the reserved
    // strip fully transparent, so the compositor background shows through and
    // the chrome reads as a small tab rather than a second titlebar.
    if spec.candidate.paints_band() {
        canvas.fill_rect(band.x, band.y, band.width, band.height, surface);
        canvas.fill_rect(band.x, band.bottom() - 1, band.width, 1, spec.theme.outline);
    }

    let radius = spec.theme.radius * f64::from(scale);
    let font_px = spec.font_px * scale;
    let tracking = spec.font_px * spec.tracking_em * scale;

    // When expanded, the tab grows to the RIGHT so action icons sit immediately
    // beside the name and share its background, rather than at the far edge.
    // The separator gets equal breathing room on both sides.
    let actions: &[Action] = if spec.expanded { &spec.actions } else { &[] };
    let icon_box = px(18, scale);
    let icon_gap = px(4, scale);
    let sep_gap = px(6, scale);
    let actions_w = if actions.is_empty() {
        0
    } else {
        // sep_gap | sep_gap, then icons each followed by a gap, minus the
        // trailing gap, plus the trailing padding.
        sep_gap * 2 + 1 + icon_box * actions.len() as u32
            + icon_gap * (actions.len() as u32 - 1)
            + px(spec.theme.side_pad, scale)
    };
    let tab = Rect::new(button.x, button.y, button.width + actions_w, button.height);

    // The wlcontrol realm-card treatment: draw the tab as an accent frame and
    // inset the fill, so the accent is thick on the left and thin elsewhere
    // while every edge follows the same rounded corners.
    if spec.candidate == Candidate::AccentFill {
        canvas.fill_round_rect(tab.x, tab.y, tab.width, tab.height, radius, button_bg);
    } else {
        let thin = px(1, scale).max(1);
        let bar = px(3, scale).max(2);
        // Three concentric shapes, so every curve is parallel to the one
        // outside it. An earlier attempt insetting the fill by different
        // amounts per side could not be parallel by construction: offsetting a
        // circular arc by unequal amounts does not yield a circular arc.
        //
        //   1. the outer shape, painted in the border colour
        //   2. the card, inset uniformly by the border width, radius R - thin
        //   3. the accent bar, the card's own left columns, clipped to the card
        //      so it follows the card's curve on its left and is straight on
        //      its right
        let hairline = spec.accent.mix(button_bg, 0.45);
        let inner_r = (radius - f64::from(thin)).max(0.0);
        canvas.fill_round_rect(tab.x, tab.y, tab.width, tab.height, radius, hairline);
        canvas.fill_round_rect(
            tab.x + thin as i32,
            tab.y + thin as i32,
            tab.width.saturating_sub(thin * 2),
            tab.height.saturating_sub(thin * 2),
            inner_r,
            button_bg,
        );
        let bar_from = tab.x + thin as i32;
        canvas.fill_round_rect_xy_clipped(
            bar_from,
            tab.y + thin as i32,
            tab.width.saturating_sub(thin * 2),
            tab.height.saturating_sub(thin * 2),
            inner_r,
            inner_r,
            inner_r,
            spec.accent,
            Some((bar_from, bar_from + bar as i32)),
        );
    }
    if spec.state.menu_open || spec.expanded {
        canvas.stroke_round_rect(
            tab.x - 1,
            tab.y - 1,
            tab.width + 2,
            tab.height + 2,
            radius + 1.0,
            spec.accent,
        );
    }

    let desired = match spec.candidate {
        Candidate::AccentFill => readable_on(button_bg),
        _ => spec.theme.foreground,
    };
    let fg = enforce_contrast(desired, button_bg, CONTRAST_TEXT_AA).unwrap_or(desired);
    let label_contrast = contrast_ratio(fg, button_bg);

    let m = fonts.measure(&spec.label, font_px, tracking);
    let text_x = tab.x + px(spec.theme.side_pad, scale) as i32;
    let baseline = tab.y + ((tab.height + m.ascent) / 2) as i32;
    for g in fonts.layout(&spec.label, font_px, tracking, text_x, baseline) {
        blend_glyph(&mut canvas.pixels, w, h, &g, fg);
    }

    // Disclosure chevron: `>` when collapsed, `<` once expanded, so the tab
    // says it can be opened and then how to close it again.
    let chev_w = px(CHEVRON_WIDTH, scale);
    let chev_x = text_x + m.width as i32 + px(5, scale) as i32;
    draw_chevron(
        &mut canvas,
        chev_x,
        tab.y + tab.height as i32 / 2,
        chev_w,
        spec.expanded,
        fg.with_alpha(0xcc),
        scale,
    );

    for (i, action) in actions.iter().enumerate() {
        if i == 0 {
            // Separator with equal space on either side.
            canvas.fill_rect(
                button.right() + sep_gap as i32,
                tab.y + px(4, scale) as i32,
                1,
                tab.height.saturating_sub(px(8, scale)),
                spec.theme.outline,
            );
        }
        let ix = button.right()
            + (sep_gap * 2 + 1) as i32
            + i as i32 * (icon_box + icon_gap) as i32;
        let iy = tab.y + ((tab.height - icon_box) / 2) as i32;
        draw_action_icon(&mut canvas, *action, ix, iy, icon_box, fg, scale);
    }

    if spec.state.keyboard_focus {
        canvas.stroke_rect(tab.x - 3, tab.y - 3, tab.width + 6, tab.height + 6, spec.theme.focus);
        canvas.stroke_rect(tab.x - 2, tab.y - 2, tab.width + 4, tab.height + 4, spec.theme.focus);
        canvas.stroke_rect(tab.x - 1, tab.y - 1, tab.width + 2, tab.height + 2, Rgba::BLACK);
    }

    // Status token: unfilled text plus a fixed glyph per capability, at lower
    // visual weight so it never competes with identity. Every concurrent
    // capability keeps its own glyph, since the shape is the non-colour
    // encoding of that capability.
    if let (Some(status), Some(rect)) = (spec.status.as_deref(), layout.status) {
        let st = spec.status_state;
        // The token opens the same menu, so it carries the same visible states
        // and the same delineation guarantee when it is being interacted with.
        if st.hover || st.pressed || st.menu_open || st.keyboard_focus {
            let mut bg = spec.theme.plate;
            if st.pressed {
                bg = bg.mix(Rgba::BLACK, 0.35);
            } else if st.menu_open {
                bg = bg.mix(Rgba::WHITE, 0.18);
            } else if st.hover {
                bg = bg.mix(Rgba::WHITE, 0.10);
            }
            canvas.fill_round_rect(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                spec.theme.radius * f64::from(scale),
                bg,
            );
            canvas.stroke_rect(rect.x, rect.y, rect.width, rect.height, spec.theme.outline);
            if st.menu_open {
                canvas.stroke_rect(rect.x - 1, rect.y - 1, rect.width + 2, rect.height + 2, spec.accent);
                canvas.stroke_rect(rect.x - 2, rect.y - 2, rect.width + 4, rect.height + 4, spec.accent);
            }
            if st.keyboard_focus {
                canvas.stroke_rect(rect.x - 3, rect.y - 3, rect.width + 6, rect.height + 6, spec.theme.focus);
                canvas.stroke_rect(rect.x - 2, rect.y - 2, rect.width + 4, rect.height + 4, spec.theme.focus);
                canvas.stroke_rect(rect.x - 1, rect.y - 1, rect.width + 2, rect.height + 2, Rgba::BLACK);
            }
        }
        let token_bg = if st.hover || st.pressed || st.menu_open || st.keyboard_focus {
            spec.theme.plate
        } else {
            surface
        };
        let sfg = enforce_contrast(spec.theme.foreground_dim, token_bg, CONTRAST_TEXT_AA)
            .unwrap_or(spec.theme.foreground);
        let sp = font_px * 0.88;
        let glyph_w = px(14, scale);
        let gap = px(5, scale);
        let sep = px(8, scale);

        // Composed tokens are separated on the wire; each part gets a glyph.
        let parts: Vec<&str> = status.split(" . ").map(str::trim).collect();
        let total: u32 = parts
            .iter()
            .map(|p| glyph_w + gap + fonts.measure(p, sp, tracking).width)
            .sum::<u32>()
            + sep * (parts.len().saturating_sub(1)) as u32;

        let mut x = if layout.reflow.status_second_row {
            rect.x
        } else {
            rect.x + ((rect.width.saturating_sub(total)) / 2) as i32
        };
        let cy = rect.y + (rect.height / 2) as i32;
        let sbase = rect.y + ((rect.height + fonts.measure("M", sp, 0.0).ascent) / 2) as i32;

        for (i, part) in parts.iter().enumerate() {
            draw_status_glyph(&mut canvas, part, x, cy, glyph_w, sfg, scale);
            let tx = x + (glyph_w + gap) as i32;
            for g in fonts.layout(part, sp, tracking, tx, sbase) {
                blend_glyph(&mut canvas.pixels, w, h, &g, sfg);
            }
            x = tx + fonts.measure(part, sp, tracking).width as i32;
            if i + 1 < parts.len() {
                x += sep as i32;
            }
        }
    }

    Rendered {
        canvas,
        layout: Some(layout),
        label_contrast,
        blocked: false,
    }
}

/// Width reserved for the disclosure chevron, in logical px.
pub const CHEVRON_WIDTH: u32 = 9;

/// A disclosure chevron: `>` when collapsed, `<` when expanded.
fn draw_chevron(
    canvas: &mut Canvas,
    x: i32,
    center_y: i32,
    width: u32,
    expanded: bool,
    color: Rgba,
    scale: f32,
) {
    let arm = ((width as f32) * 0.55).round() as i32;
    let thick = ((1.4 * scale).round() as u32).max(1);
    for i in 0..=arm {
        // Collapsed points right, expanded points left.
        let dx = if expanded { arm - i } else { i };
        canvas.fill_rect(x + dx, center_y - arm + i, thick, thick, color);
        canvas.fill_rect(x + dx, center_y + arm - i, thick, thick, color);
    }
}

/// Procedural action icons, drawn by the trusted renderer so the action set
/// cannot be restyled into something it is not.
///
/// Every icon is composed on a 20x20 reference grid and scaled once, so shapes
/// stay proportional instead of drifting as the box size changes.
fn draw_action_icon(
    canvas: &mut Canvas,
    action: Action,
    x: i32,
    y: i32,
    box_size: u32,
    color: Rgba,
    _scale: f32,
) {
    const GRID: f32 = 20.0;
    let k = box_size as f32 / GRID;
    // Grid-space helpers: position and length in reference units.
    let p = |v: f32| (v * k).round() as i32;
    let l = |v: f32| ((v * k).round() as u32).max(1);
    let ox = x;
    let oy = y;
    let rect = |c: &mut Canvas, gx: f32, gy: f32, gw: f32, gh: f32| {
        c.fill_rect(ox + p(gx), oy + p(gy), l(gw), l(gh), color);
    };
    let stroke = |c: &mut Canvas, gx: f32, gy: f32, gw: f32, gh: f32, r: f32| {
        c.stroke_round_rect(ox + p(gx), oy + p(gy), l(gw), l(gh), f64::from(r * k), color);
    };

    match action {
        Action::Terminal => {
            stroke(canvas, 2.0, 3.0, 16.0, 14.0, 2.0);
            // A chevron built from two diagonals, then a caret rule.
            for i in 0..4 {
                let d = i as f32;
                rect(canvas, 5.0 + d, 7.0 + d, 1.5, 1.5);
            }
            for i in 0..4 {
                let d = i as f32;
                rect(canvas, 8.0 - d, 11.0 + d, 1.5, 1.5);
            }
            rect(canvas, 10.5, 12.5, 5.0, 1.5);
        }
        Action::Audio => {
            // Cone: a stepped triangle, so it stays clean at any size.
            rect(canvas, 4.0, 8.0, 3.0, 4.0);
            for i in 0..4 {
                let d = i as f32;
                rect(canvas, 7.0 + d, 6.5 + d * 0.9, 1.2, 7.0 - d * 1.8);
            }
            // Two arcs as short vertical strokes, not a sampled circle.
            rect(canvas, 13.0, 8.0, 1.2, 4.0);
            rect(canvas, 15.2, 6.5, 1.2, 7.0);
        }
        Action::Usb => {
            rect(canvas, 9.4, 4.0, 1.4, 12.0);
            stroke(canvas, 8.2, 2.4, 3.8, 3.8, 1.9);
            rect(canvas, 5.0, 9.0, 9.0, 1.4);
            rect(canvas, 5.0, 9.0, 1.4, 3.0);
            rect(canvas, 12.6, 11.0, 2.8, 2.8);
        }
        Action::Info => {
            stroke(canvas, 2.5, 2.5, 15.0, 15.0, 7.5);
            rect(canvas, 9.3, 6.0, 1.6, 1.8);
            rect(canvas, 9.3, 9.0, 1.6, 5.0);
        }
        Action::Stop => {
            rect(canvas, 5.5, 5.5, 9.0, 9.0);
        }
    }
}

/// Fixed trusted glyphs for the status token. Drawn procedurally so the shape
/// is part of the trusted renderer rather than a themeable icon: the glyph is
/// the redundant, non-colour encoding of a security-capability condition.
fn draw_status_glyph(
    canvas: &mut Canvas,
    status: &str,
    x: i32,
    center_y: i32,
    box_w: u32,
    color: Rgba,
    scale: f32,
) {
    let s = |v: f32| ((v * scale).round() as i32).max(1);
    let unit = |v: f32| ((v * scale).round() as u32).max(1);

    if status.starts_with("MIC") {
        // Capsule body, stand, and base.
        let cw = unit(6.0);
        let ch = unit(9.0);
        let cx = x + (box_w as i32 - cw as i32) / 2;
        let cy = center_y - s(7.0);
        canvas.fill_round_rect(cx, cy, cw, ch, f64::from(cw) / 2.0, color);
        canvas.fill_rect(cx + (cw as i32 / 2) - s(0.5), cy + ch as i32, unit(1.5), unit(3.0), color);
        canvas.fill_rect(cx - s(2.0), cy + ch as i32 + s(3.0), cw + unit(4.0), unit(1.5), color);
        // A slash marks the muted variant: shape, not hue, carries the state.
        if status.contains("MUTED") {
            let n = s(14.0);
            for i in 0..n {
                canvas.blend(x + i, center_y - s(7.0) + i, color);
                canvas.blend(x + i + 1, center_y - s(7.0) + i, color);
            }
        }
    } else if status.starts_with("USB") {
        // Stem with a trident head.
        let cx = x + box_w as i32 / 2;
        let top = center_y - s(7.0);
        canvas.fill_rect(cx - s(0.5), top, unit(1.5), unit(14.0), color);
        canvas.fill_round_rect(cx - s(2.0), top, unit(4.0), unit(4.0), 2.0, color);
        canvas.fill_rect(cx - s(5.0), top + s(6.0), unit(10.0), unit(1.5), color);
        canvas.fill_rect(cx - s(5.0), top + s(6.0), unit(1.5), unit(4.0), color);
        canvas.fill_round_rect(cx + s(3.0), top + s(9.0), unit(3.0), unit(3.0), 1.5, color);
    } else {
        // Generic attention mark for DEGRADED / STOPPING and future states.
        let cx = x + box_w as i32 / 2;
        let top = center_y - s(7.0);
        canvas.fill_rect(cx - s(0.5), top, unit(1.5), unit(9.0), color);
        canvas.fill_rect(cx - s(0.5), top + s(11.0), unit(1.5), unit(2.0), color);
    }
}

/// A deliberate blocked mark for windows too narrow for legible text: a barred
/// circle. Shape, never hue, and never an ellipsis that reads as corruption.
fn draw_blocked_glyph(canvas: &mut Canvas, cx: i32, cy: i32, scale: f32, color: Rgba) {
    let r = ((14.0 * scale).round() as i32).max(6);
    let thick = ((2.0 * scale).round() as u32).max(1);
    // Ring, drawn as a filled disc minus an inner disc.
    for dy in -r..=r {
        for dx in -r..=r {
            let d2 = dx * dx + dy * dy;
            let outer = r * r;
            let inner = (r - thick as i32) * (r - thick as i32);
            if d2 <= outer && d2 >= inner {
                canvas.blend(cx + dx, cy + dy, color);
            }
        }
    }
    // Diagonal bar.
    for i in -r..=r {
        for t in 0..thick as i32 {
            canvas.blend(cx + i, cy + i + t, color);
        }
    }
}

/// Fail-closed: obscure the guest and state why. Never a bare window.
///
/// The trusted band is retained above the blocked area. Removing it would make
/// the learned trusted location disappear exactly when it matters most, and
/// would leave full-window artwork a guest could imitate.
fn render_blocked(spec: &ChromeSpec, fonts: &TextRenderer) -> Rendered {
    let scale = spec.scale;
    let w = px(spec.content_width, scale).max(1) as usize;
    let content_h = px(spec.content_height, scale).max(1) as usize;
    let band_h = px(crate::geom::MIN_BAND_HEIGHT, scale) as usize;
    let h = band_h + content_h;

    let mut canvas = Canvas::new(w, h, spec.theme.surface);

    // Trusted band, in the same place as always.
    canvas.fill_rect(0, 0, w as u32, band_h as u32, spec.theme.surface);
    canvas.fill_rect(0, band_h as i32 - 1, w as u32, 1, spec.theme.outline);

    let font_px = spec.font_px * scale;
    let fg = enforce_contrast(spec.theme.foreground, spec.theme.surface, CONTRAST_TEXT_AA)
        .unwrap_or(spec.theme.foreground);

    // Identity slot carries UNVERIFIED, with no realm accent anywhere. In a
    // window too narrow to hold it, the plate is omitted rather than allowed
    // to overflow: the blocked state must not itself render broken.
    let pad = px(spec.theme.side_pad, scale);
    let label = "UNVERIFIED";
    let m = fonts.measure(label, font_px, 0.0);
    let plate_w = m.width + pad * 2;
    if plate_w + pad * 2 <= w as u32 {
        let plate_h = px(24, scale).min(band_h as u32);
        let plate_y = ((band_h as u32 - plate_h) / 2) as i32;
        canvas.fill_round_rect(
            pad as i32,
            plate_y,
            plate_w,
            plate_h,
            spec.theme.radius * f64::from(scale),
            spec.theme.plate,
        );
        canvas.stroke_rect(pad as i32, plate_y, plate_w, plate_h, spec.theme.outline);
        // A hatched rule stands in for the accent rule: a pattern, never a hue.
        let rule = px(spec.theme.accent_rule, scale).max(1);
        for i in 0..plate_w {
            if (i / 2) % 2 == 0 {
                canvas.fill_rect(
                    pad as i32 + i as i32,
                    plate_y + plate_h as i32 - rule as i32,
                    1,
                    rule,
                    fg,
                );
            }
        }
        let baseline = plate_y + ((plate_h.saturating_sub(rule) + m.ascent) / 2) as i32;
        for g in fonts.layout(label, font_px, 0.0, (pad * 2) as i32, baseline) {
            blend_glyph(&mut canvas.pixels, w, h, &g, fg);
        }
    }

    // Blocked content: a diagonal hatch, so the state survives grayscale and
    // colour-vision deficiency without depending on any hue.
    let hatch = spec.theme.foreground.with_alpha(0x1c);
    let step = px(10, scale).max(4) as usize;
    for y in band_h..h {
        for x in 0..w {
            if (x + y) % step == 0 {
                canvas.blend(x as i32, y as i32, hatch);
            }
        }
    }

    // The explanation is shown only when it fits legibly. Below that width an
    // ellipsis would read as rendering corruption, so a deliberate blocked
    // glyph carries the state instead; the full text remains available to AT.
    let budget = (w as u32).saturating_sub(pad * 2);
    let full = "content blocked - identity could not be verified";
    let full_w = fonts.measure(full, font_px, 0.0).width;
    let short = "blocked";
    let short_w = fonts.measure(short, font_px, 0.0).width;

    let content_mid = (band_h + content_h / 2) as i32;
    if full_w <= budget {
        let mx = ((w as i32) - (full_w as i32)) / 2;
        for g in fonts.layout(full, font_px, 0.0, mx, content_mid) {
            blend_glyph(&mut canvas.pixels, w, h, &g, fg);
        }
    } else if short_w <= budget {
        let mx = ((w as i32) - (short_w as i32)) / 2;
        for g in fonts.layout(short, font_px, 0.0, mx, content_mid) {
            blend_glyph(&mut canvas.pixels, w, h, &g, fg);
        }
    } else {
        draw_blocked_glyph(&mut canvas, (w / 2) as i32, content_mid, scale, fg);
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

    /// The blocked state must not itself render broken in a tiny window.
    #[test]
    fn blocked_state_does_not_overflow_a_narrow_window() {
        let f = fonts();
        for width in [60_u32, 90, 150, 300, 880] {
            let mut s = spec(Candidate::BandNeutral);
            s.identity_verified = false;
            s.content_width = width;
            let r = render(&s, &f, DARK);
            assert!(r.blocked);
            assert_eq!(
                r.canvas.width, width as usize,
                "blocked canvas must match the window width"
            );
            // Every pixel is inside the canvas by construction; the real check
            // is that rendering a very narrow blocked window does not panic and
            // still paints the hatch.
            assert!(r.canvas.pixels.iter().any(|p| p.a > 0));
        }
    }

    /// The trusted band must survive the blocked state: removing it would make
    /// the learned trusted location vanish exactly when it matters most.
    #[test]
    fn blocked_state_retains_the_trusted_band() {
        let mut s = spec(Candidate::BandNeutral);
        s.identity_verified = false;
        let r = render(&s, &fonts(), DARK);
        let band_h = crate::geom::MIN_BAND_HEIGHT as usize;
        // The band's plate region is painted, not hatched content.
        let plate = r.canvas.get(14, band_h / 2);
        assert!(plate.a > 0);
        assert_ne!(
            plate, s.theme.surface,
            "the identity plate must be visible inside the band"
        );
        // Total height includes the band above the content.
        assert!(r.canvas.height > s.content_height as usize);
    }

    /// The plate must delineate the target on its own, without help from the
    /// accent, and must keep doing so in grayscale. WCAG 1.4.11 asks for 3:1
    /// on the component boundary, which the outline carries here so the fill
    /// can stay visually quiet.
    #[test]
    fn identity_target_is_delineated_against_the_band() {
        let t = Theme::default();
        let ratio = t.target_delineation();
        assert!(
            ratio >= crate::color::CONTRAST_NON_TEXT_AA,
            "target delineation measured {ratio:.2}, below the 3:1 floor"
        );

        // The same must hold without hue, for monochrome displays.
        let gray = contrast_ratio(t.plate.to_grayscale(), t.surface.to_grayscale()).max(
            contrast_ratio(
                t.composited_outline().to_grayscale(),
                t.surface.to_grayscale(),
            ),
        );
        assert!(
            gray >= crate::color::CONTRAST_NON_TEXT_AA,
            "grayscale delineation measured {gray:.2}"
        );
    }

    /// Menu-open is a held state and must not be mistakable for merely focused.
    #[test]
    fn menu_open_is_visually_distinct_from_focused() {
        let f = fonts();
        let base = render(&spec(Candidate::BandNeutral), &f, DARK);
        let mut open_spec = spec(Candidate::BandNeutral);
        open_spec.state = VisualState {
            menu_open: true,
            ..VisualState::focused()
        };
        let open = render(&open_spec, &f, DARK);

        let lb = base.layout.unwrap();
        let differing = (0..lb.band.height as usize)
            .flat_map(|y| (0..lb.outer.width as usize).map(move |x| (x, y)))
            .filter(|&(x, y)| base.canvas.get(x, y) != open.canvas.get(x, y))
            .count();
        assert!(
            differing > 100,
            "menu-open differed in only {differing} pixels"
        );
    }

    #[test]
    fn hover_and_pressed_are_distinguishable_from_each_other() {
        let f = fonts();
        let mut hov = spec(Candidate::BandNeutral);
        hov.state = VisualState {
            hover: true,
            ..VisualState::focused()
        };
        let mut prs = spec(Candidate::BandNeutral);
        prs.state = VisualState {
            pressed: true,
            ..VisualState::focused()
        };
        let a = render(&hov, &f, DARK);
        let b = render(&prs, &f, DARK);
        let l = a.layout.unwrap();
        // Sample inside the plate fill, past the accent edge and the outline.
        let sx = (l.button.x + 10) as usize;
        let sy = (l.button.y + l.button.height as i32 / 2) as usize;
        let p1 = a.canvas.get(sx, sy);
        let p2 = b.canvas.get(sx, sy);
        assert_ne!(p1, p2, "hover and pressed must not render identically");
        // And both must remain distinguishable without hue.
        assert_ne!(p1.to_grayscale(), p2.to_grayscale());
    }

    #[test]
    fn status_glyph_is_drawn_alongside_the_words() {
        let f = fonts();
        for token in ["MIC", "MIC MUTED", "USB", "DEGRADED"] {
            let mut s = spec(Candidate::BandNeutral);
            s.status = Some(token.to_owned());
            let r = render(&s, &f, DARK);
            let l = r.layout.unwrap();
            let rect = l.status.expect("token present");
            // The leading third of the token area holds the glyph, so it must
            // contain ink beyond the bare band surface.
            let ink = (rect.x..rect.x + (rect.width / 3) as i32)
                .flat_map(|x| (rect.y..rect.bottom()).map(move |y| (x, y)))
                .any(|(x, y)| {
                    let p = r.canvas.get(x as usize, y as usize);
                    p != s.theme.surface && p.a > 0
                });
            assert!(ink, "{token} rendered no glyph");
        }
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
