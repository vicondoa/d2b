//! Geometry for the window identity tab.
//!
//! Two invariants drive everything here:
//!
//! 1. The pointer input region must cover the identity button and nothing else.
//!    The shipping rail claims the whole left window edge and swallows those
//!    events, which is why edge interaction is currently impossible.
//! 2. That region must be at least 32x32 logical px and must sit entirely
//!    inside proxy-owned chrome, never overlapping guest content.

/// Minimum pointer target, per WCAG 2.2 SC 2.5.8 plus the panel's Fitts's-law
/// uplift from 24 to 32.
pub const MIN_TARGET: u32 = 32;
/// Minimum visible (drawn) height of the identity button.
pub const MIN_VISIBLE_FACE: u32 = 24;
/// Minimum reserved chrome band height, in logical px.
///
/// This is a floor, not a fixed height. Every UX seat independently found that
/// a fixed band cannot hold a wrapped label or text at 200% scaling, so the
/// band grows from measured content and identity is never clipped to fit.
pub const MIN_BAND_HEIGHT: u32 = 32;

/// Breathing room above and below the visible tab, in logical px. Kept at one
/// pixel so the tab sits close to guest content rather than floating in the
/// reserved strip; the strip itself stays large enough for the pointer target.
pub const TAB_INSET: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(self) -> i32 {
        self.x.saturating_add(self.width as i32)
    }

    pub fn bottom(self) -> i32 {
        self.y.saturating_add(self.height as i32)
    }

    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    pub fn intersects(self, other: Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// Grow on every side, clamped at zero so a rect never inverts.
    pub fn inflate(self, by: u32) -> Rect {
        let by_i = by as i32;
        Rect {
            x: self.x - by_i,
            y: self.y - by_i,
            width: self.width + by * 2,
            height: self.height + by * 2,
        }
    }
}

/// Where the reserved chrome band sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BandPlacement {
    Top,
    Bottom,
}

/// Resolved layout for one decorated window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChromeLayout {
    /// Guest content size, unchanged by chrome.
    pub content: Size,
    /// Full wrapper size the proxy declares as its window geometry.
    pub outer: Size,
    /// Where the guest subsurface is placed inside the wrapper.
    pub content_origin: (i32, i32),
    /// The reserved chrome band.
    pub band: Rect,
    /// The identity button: the only region that takes pointer input.
    pub button: Rect,
    /// Optional drag-move region: the band minus the button and status token.
    pub drag: Option<Rect>,
    /// Right-aligned status token, when one is shown.
    pub status: Option<Rect>,
    /// How the layout degraded to fit, for honest reporting and tests.
    pub reflow: Reflow,
}

/// The steps a layout took to fit, in the order the panel fixed:
/// short name, then wrap, then grow the band. Identity always wins, and
/// security-capability state is never dropped — it moves to a second row and
/// grows the band rather than disappearing into the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Reflow {
    pub wrapped_label: bool,
    /// The status token moved below identity instead of sitting beside it.
    pub status_second_row: bool,
    pub grew_band: bool,
}

/// Inputs to layout resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutInput {
    pub content: Size,
    pub placement: BandPlacement,
    /// Measured width of the identity button's visible face.
    pub button_width: u32,
    /// Measured height the label block needs, including wrapped lines.
    pub label_block_height: u32,
    /// True when the label had to wrap to fit the available width.
    pub label_wrapped: bool,
    /// Measured width of the status token; zero when no token is shown.
    pub status_width: u32,
    /// Inset from the band's left/right edges.
    pub side_pad: u32,
    /// Vertical padding above and below the label block.
    pub vertical_pad: u32,
    /// Thickness of the accent rule under the identity button.
    pub accent_rule: u32,
    /// Whether host-verified identity is available for this window.
    pub identity_verified: bool,
}

impl Default for LayoutInput {
    fn default() -> Self {
        Self {
            content: Size::new(800, 600),
            placement: BandPlacement::Top,
            button_width: 96,
            label_block_height: 18,
            label_wrapped: false,
            status_width: 0,
            side_pad: 8,
            vertical_pad: 2,
            accent_rule: 4,
            identity_verified: true,
        }
    }
}

/// The outcome of resolving chrome for a window.
///
/// There is deliberately no "undecorated" outcome. Guest content is never
/// mapped without persistent trusted identity: when chrome cannot be drawn
/// compliantly, the proxy blocks rather than degrading to a bare window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromeOutcome {
    /// Chrome resolved; decorate normally.
    Decorate(ChromeLayout),
    /// Chrome cannot be drawn compliantly. Obscure the guest and block input
    /// behind the accessible host interstitial; never show bare guest content.
    FailClosed(FailClosedReason),
}

/// Why chrome could not be drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailClosedReason {
    /// The window has no drawable content area.
    EmptyContent,
    /// Even the compact display name cannot fit the enforced minimum width.
    IdentityDoesNotFit,
    /// Host-verified identity was unavailable.
    UnverifiedIdentity,
}

impl ChromeOutcome {
    pub fn layout(self) -> Option<ChromeLayout> {
        match self {
            Self::Decorate(l) => Some(l),
            Self::FailClosed(_) => None,
        }
    }

    /// True when guest content must be obscured and input blocked.
    pub fn blocks_guest(self) -> bool {
        matches!(self, Self::FailClosed(_))
    }
}

/// The band height this content requires: the floor, or more if measured
/// content needs it. Never less than the floor, never clipping content.
pub fn required_band_height(input: LayoutInput) -> u32 {
    band_height_for(input, false)
}

/// The height of one row inside the band. A row is never shorter than the
/// visible-face floor, so the band is derived from rows rather than the other
/// way round; otherwise a forced-up face overflows its own band.
fn row_height(input: LayoutInput) -> u32 {
    MIN_VISIBLE_FACE.max(input.label_block_height)
}

/// Band height, accounting for a status token that had to move to a second row.
///
/// The band is padding plus rows. It carries no extra allowance for the accent,
/// which is drawn as an edge on the tab itself rather than a rule beneath it,
/// so the gap between chrome and guest content stays as small as the padding.
fn band_height_for(input: LayoutInput, status_second_row: bool) -> u32 {
    let row = row_height(input);
    let rows = if status_second_row {
        row.saturating_mul(2).saturating_add(input.vertical_pad)
    } else {
        row
    };
    rows.saturating_add(input.vertical_pad.saturating_mul(2))
        .max(MIN_BAND_HEIGHT)
}

/// The visible tab height for a resolved band.
///
/// The tab takes the whole band apart from one pixel of breathing room above
/// and below, so the gap to guest content is as small as the design allows
/// while the reserved strip still holds a compliant pointer target.
fn face_height(band_height: u32, input: LayoutInput, status_second_row: bool) -> u32 {
    if status_second_row {
        row_height(input)
    } else {
        band_height
            .saturating_sub(TAB_INSET * 2)
            .max(MIN_VISIBLE_FACE)
            .min(band_height)
    }
}

/// Resolve chrome layout, or fail closed.
///
/// Failing closed is the deliberate terminal case of the reflow order: short
/// name, wrap, grow the band, and finally refuse. Refusal blocks the guest; it
/// never yields an undecorated window.
pub fn resolve(input: LayoutInput) -> ChromeOutcome {
    if input.content.is_empty() {
        return ChromeOutcome::FailClosed(FailClosedReason::EmptyContent);
    }
    if !input.identity_verified {
        return ChromeOutcome::FailClosed(FailClosedReason::UnverifiedIdentity);
    }

    // Identity has absolute priority. If even the compact name cannot fit the
    // enforced minimum width, fail closed rather than clip or shrink it.
    let avail = input.content.width.saturating_sub(input.side_pad * 2);
    let button_w = input.button_width.max(MIN_TARGET);
    if avail < button_w {
        return ChromeOutcome::FailClosed(FailClosedReason::IdentityDoesNotFit);
    }

    // Security-capability state is never dropped. If it cannot sit beside
    // identity, it moves to a second row and the band grows to hold it.
    let status_second_row =
        input.status_width > 0 && avail < button_w + input.status_width + input.side_pad;
    if status_second_row && avail < input.status_width {
        // Not even a dedicated row can hold it: refuse rather than hide it.
        return ChromeOutcome::FailClosed(FailClosedReason::IdentityDoesNotFit);
    }

    let band_height = band_height_for(input, status_second_row);
    let reflow = Reflow {
        wrapped_label: input.label_wrapped,
        status_second_row,
        grew_band: band_height > MIN_BAND_HEIGHT,
    };

    let Some(outer_h) = input.content.height.checked_add(band_height) else {
        return ChromeOutcome::FailClosed(FailClosedReason::EmptyContent);
    };
    let outer = Size::new(input.content.width, outer_h);

    let (band_y, content_y) = match input.placement {
        BandPlacement::Top => (0_i32, band_height as i32),
        BandPlacement::Bottom => (input.content.height as i32, 0_i32),
    };
    let band = Rect::new(0, band_y, outer.width, band_height);

    // Rows are sized first; the band was derived from them, so a row always
    // fits inside it. Padding above and below is equal by construction.
    let face_h = face_height(band_height, input, status_second_row);
    let rows_total = if status_second_row {
        face_h * 2 + input.vertical_pad
    } else {
        face_h
    };
    let slack = band_height.saturating_sub(rows_total);
    let face_y = band_y + (slack / 2) as i32;

    let button = Rect::new(input.side_pad as i32, face_y, button_w, face_h);

    let status = (input.status_width > 0).then(|| {
        if status_second_row {
            Rect::new(
                input.side_pad as i32,
                face_y + face_h as i32 + input.vertical_pad as i32,
                input.status_width,
                face_h,
            )
        } else {
            Rect::new(
                (outer.width - input.side_pad - input.status_width) as i32,
                face_y,
                input.status_width,
                face_h,
            )
        }
    });

    // The drag region is whatever the first row does not claim.
    let drag_x = button.right();
    let drag_end = match status {
        Some(s) if !status_second_row => s.x,
        _ => (outer.width - input.side_pad) as i32,
    };
    let drag = (drag_end > drag_x)
        .then(|| Rect::new(drag_x, band_y, (drag_end - drag_x) as u32, face_h));

    ChromeOutcome::Decorate(ChromeLayout {
        content: input.content,
        outer,
        content_origin: (0, content_y),
        band,
        button,
        drag,
        status,
        reflow,
    })
}

impl ChromeLayout {
    /// The pointer input region the wrapper surface claims.
    ///
    /// Always at least `MIN_TARGET` on both axes, and always clamped inside the
    /// reserved band. The clamp is the point: the shipping rail's region spills
    /// across the window edge, which is why edge interaction is swallowed
    /// today. This region can never reach guest content or a window edge.
    pub fn input_region(&self) -> Rect {
        let mut r = self.button;

        if r.height < MIN_TARGET {
            let grow = MIN_TARGET - r.height;
            r = Rect::new(r.x, r.y - (grow / 2) as i32, r.width, MIN_TARGET);
        }
        if r.width < MIN_TARGET {
            r = Rect::new(r.x, r.y, MIN_TARGET, r.height);
        }

        // Clamp vertically into the band.
        if r.y < self.band.y {
            r = Rect::new(r.x, self.band.y, r.width, r.height);
        }
        if r.bottom() > self.band.bottom() {
            let overflow = (r.bottom() - self.band.bottom()) as u32;
            let shifted = r.y - overflow as i32;
            r = if shifted >= self.band.y {
                Rect::new(r.x, shifted, r.width, r.height)
            } else {
                // The band itself is the limit; take all of it and no more.
                Rect::new(r.x, self.band.y, r.width, self.band.height)
            };
        }

        // Clamp horizontally inside the window.
        if r.right() > self.outer.width as i32 {
            let overflow = (r.right() - self.outer.width as i32) as u32;
            r = Rect::new(r.x - overflow as i32, r.y, r.width, r.height);
        }
        if r.x < 0 {
            r = Rect::new(0, r.y, r.width, r.height);
        }
        r
    }

    /// The guest content rect in wrapper-local coordinates.
    pub fn content_rect(&self) -> Rect {
        Rect::new(
            self.content_origin.0,
            self.content_origin.1,
            self.content.width,
            self.content.height,
        )
    }

    /// Vertical cost of chrome, for honest layout accounting.
    pub fn geometry_cost(&self) -> u32 {
        self.outer.height - self.content.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_band_offsets_content_and_costs_its_height() {
        let l = resolve(LayoutInput::default()).layout().unwrap();
        assert_eq!(l.band, Rect::new(0, 0, 800, 32));
        assert_eq!(l.content_origin, (0, 32));
        assert_eq!(l.outer, Size::new(800, 632));
        assert_eq!(l.geometry_cost(), 32);
    }

    #[test]
    fn bottom_band_leaves_content_at_origin() {
        let l = resolve(LayoutInput {
            placement: BandPlacement::Bottom,
            ..Default::default()
        })
        .layout()
        .unwrap();
        assert_eq!(l.content_origin, (0, 0));
        assert_eq!(l.band.y, 600);
        assert_eq!(l.geometry_cost(), 32);
    }

    /// The core regression: input must NOT cover the window edges. This is the
    /// exact defect in the shipping rail.
    #[test]
    fn input_region_never_covers_the_window_edges() {
        let l = resolve(LayoutInput::default()).layout().unwrap();
        let r = l.input_region();
        assert!(r.x > 0, "must not touch the left edge");
        assert!(
            r.right() < l.outer.width as i32,
            "must not touch the right edge"
        );
        assert!(
            r.bottom() <= l.band.bottom(),
            "must stay inside the reserved band"
        );
        assert!(
            !r.intersects(l.content_rect()),
            "must never overlap guest content"
        );
    }

    #[test]
    fn input_region_meets_the_target_floor_across_sizes() {
        for label_h in [14_u32, 18, 22, 30, 40] {
            for button_width in [MIN_TARGET, 48, 96, 200] {
                let l = resolve(LayoutInput {
                    label_block_height: label_h,
                    button_width,
                    ..Default::default()
                })
                .layout()
                .unwrap();
                let r = l.input_region();
                assert!(r.width >= MIN_TARGET, "label {label_h}: width {}", r.width);
                assert!(r.height >= MIN_TARGET, "label {label_h}: height {}", r.height);
                assert!(!r.intersects(l.content_rect()));
            }
        }
    }

    #[test]
    fn visible_face_meets_its_own_floor() {
        let l = resolve(LayoutInput::default()).layout().unwrap();
        assert!(l.button.height >= MIN_VISIBLE_FACE);
        assert!(l.button.height <= l.band.height);
    }

    // ---- responsive band height (the unanimous round-2 finding) ----

    #[test]
    fn band_never_falls_below_the_floor() {
        // A tiny label must not produce a sub-floor band.
        let l = resolve(LayoutInput {
            label_block_height: 4,
            vertical_pad: 0,
            accent_rule: 0,
            ..Default::default()
        })
        .layout()
        .unwrap();
        assert_eq!(l.band.height, MIN_BAND_HEIGHT);
        assert!(!l.reflow.grew_band);
    }

    #[test]
    fn band_grows_for_a_wrapped_two_line_label() {
        // Two 14px lines at 1.3 line-height plus padding and rule exceeds 32.
        let two_lines = 36;
        let input = LayoutInput {
            label_block_height: two_lines,
            label_wrapped: true,
            ..Default::default()
        };
        let l = resolve(input).layout().unwrap();
        assert!(l.band.height > MIN_BAND_HEIGHT, "band was {}", l.band.height);
        assert_eq!(l.band.height, required_band_height(input));
        assert!(l.band.height >= two_lines, "the label block must fit");
        assert!(l.reflow.grew_band);
        assert!(l.reflow.wrapped_label);
        // Content is pushed down by exactly the band height: nothing is clipped.
        assert_eq!(l.content_origin.1, l.band.height as i32);
        assert_eq!(l.geometry_cost(), l.band.height);
    }

    #[test]
    fn band_grows_for_two_hundred_percent_text() {
        // 14px -> 28px scaled single line.
        let l = resolve(LayoutInput {
            label_block_height: 34,
            ..Default::default()
        })
        .layout()
        .unwrap();
        assert!(l.band.height >= 34, "label must never be clipped");
        assert!(l.reflow.grew_band);
    }

    #[test]
    fn required_band_height_is_monotonic_in_label_height() {
        let mut prev = 0;
        for h in [4_u32, 12, 18, 24, 30, 36, 48, 64] {
            let got = required_band_height(LayoutInput {
                label_block_height: h,
                ..Default::default()
            });
            assert!(got >= prev, "height must never shrink as content grows");
            assert!(got >= MIN_BAND_HEIGHT);
            prev = got;
        }
    }

    // ---- reflow: identity wins, capability state never disappears ----

    /// Security-capability state must never be hidden. When it cannot sit
    /// beside identity it moves to a second row and the band grows.
    #[test]
    fn narrow_window_moves_status_to_a_second_row_rather_than_dropping_it() {
        let wide = resolve(LayoutInput {
            status_width: 60,
            ..Default::default()
        })
        .layout()
        .unwrap();
        assert!(!wide.reflow.status_second_row, "a wide band keeps one row");

        let narrow = resolve(LayoutInput {
            content: Size::new(140, 400),
            button_width: 96,
            status_width: 60,
            ..Default::default()
        })
        .layout()
        .unwrap();
        let status = narrow
            .status
            .expect("capability state must never be dropped");
        assert!(narrow.reflow.status_second_row);
        assert!(narrow.reflow.grew_band, "a second row must grow the band");
        assert!(narrow.band.height > wide.band.height);
        assert!(!status.intersects(narrow.button));
        assert!(!status.intersects(narrow.content_rect()));
        assert!(
            status.y > narrow.button.y,
            "the second row sits below identity"
        );
        assert_eq!(narrow.button.width, 96, "identity keeps its width");
    }

    #[test]
    fn status_is_never_silently_hidden_at_any_width() {
        for width in [120_u32, 140, 200, 400, 800] {
            let outcome = resolve(LayoutInput {
                content: Size::new(width, 400),
                button_width: 96,
                status_width: 60,
                ..Default::default()
            });
            match outcome {
                ChromeOutcome::Decorate(l) => assert!(
                    l.status.is_some(),
                    "width {width} decorated but dropped capability state"
                ),
                // Refusing is acceptable; hiding is not.
                ChromeOutcome::FailClosed(_) => {}
            }
        }
    }

    /// Failing closed must block the guest, never yield a bare window.
    /// This is the terminal case of the reflow order.
    #[test]
    fn fails_closed_when_even_the_identity_button_cannot_fit() {
        let outcome = resolve(LayoutInput {
            content: Size::new(40, 400),
            button_width: 96,
            ..Default::default()
        });
        assert_eq!(
            outcome,
            ChromeOutcome::FailClosed(FailClosedReason::IdentityDoesNotFit)
        );
        assert!(outcome.blocks_guest(), "guest must be blocked, not bare");
        assert!(outcome.layout().is_none());
    }

    #[test]
    fn fails_closed_on_empty_content() {
        let outcome = resolve(LayoutInput {
            content: Size::new(0, 600),
            ..Default::default()
        });
        assert_eq!(
            outcome,
            ChromeOutcome::FailClosed(FailClosedReason::EmptyContent)
        );
        assert!(outcome.blocks_guest());
    }

    #[test]
    fn fails_closed_when_identity_is_unverified() {
        let outcome = resolve(LayoutInput {
            identity_verified: false,
            ..Default::default()
        });
        assert_eq!(
            outcome,
            ChromeOutcome::FailClosed(FailClosedReason::UnverifiedIdentity)
        );
        assert!(
            outcome.blocks_guest(),
            "unverified identity must obscure the guest and block input"
        );
    }

    /// There is no code path that produces an undecorated mapped window: every
    /// outcome either carries chrome or blocks the guest.
    #[test]
    fn every_outcome_either_decorates_or_blocks() {
        let cases = [
            LayoutInput::default(),
            LayoutInput {
                content: Size::new(0, 0),
                ..Default::default()
            },
            LayoutInput {
                content: Size::new(40, 400),
                ..Default::default()
            },
            LayoutInput {
                identity_verified: false,
                ..Default::default()
            },
            LayoutInput {
                label_block_height: 96,
                ..Default::default()
            },
        ];
        for input in cases {
            match resolve(input) {
                ChromeOutcome::Decorate(l) => {
                    assert!(l.band.height >= MIN_BAND_HEIGHT);
                    assert!(!resolve(input).blocks_guest());
                }
                ChromeOutcome::FailClosed(_) => {
                    assert!(resolve(input).blocks_guest());
                }
            }
        }
    }

    #[test]
    fn status_sits_right_aligned_and_never_overlaps_identity() {
        let l = resolve(LayoutInput {
            content: Size::new(800, 600),
            button_width: 96,
            status_width: 72,
            ..Default::default()
        })
        .layout()
        .unwrap();
        let s = l.status.expect("wide window keeps the status token");
        assert_eq!(s.right(), (800 - 8) as i32);
        assert!(!s.intersects(l.button));
        assert!(!s.intersects(l.content_rect()));
        assert!(!l.reflow.status_second_row);
    }

    #[test]
    fn drag_region_excludes_button_status_and_content() {
        let l = resolve(LayoutInput {
            status_width: 72,
            ..Default::default()
        })
        .layout()
        .unwrap();
        let d = l.drag.expect("wide band has a drag strip");
        assert!(!d.intersects(l.button));
        assert!(!d.intersects(l.status.unwrap()));
        assert!(!d.intersects(l.content_rect()));
    }

    #[test]
    fn drag_region_absent_when_the_button_fills_the_band() {
        let l = resolve(LayoutInput {
            content: Size::new(120, 400),
            button_width: 104,
            side_pad: 8,
            ..Default::default()
        })
        .layout()
        .unwrap();
        assert!(l.drag.is_none());
    }

    #[test]
    fn rect_contains_is_half_open() {
        let r = Rect::new(10, 10, 5, 5);
        assert!(r.contains(10, 10));
        assert!(r.contains(14, 14));
        assert!(!r.contains(15, 14), "right edge is exclusive");
        assert!(!r.contains(14, 15), "bottom edge is exclusive");
        assert!(!r.contains(9, 10));
    }

    /// The panel required this cost to be stated rather than hidden.
    #[test]
    fn stacked_column_cost_is_n_times_band_height() {
        let single = resolve(LayoutInput::default()).layout().unwrap();
        assert_eq!(single.geometry_cost() * 6, 192);

        // Under growth the cost rises with it, per window.
        let grown = resolve(LayoutInput {
            label_block_height: 36,
            label_wrapped: true,
            ..Default::default()
        })
        .layout()
        .unwrap();
        assert!(grown.geometry_cost() * 6 > 192);
    }
}
