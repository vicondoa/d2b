//! Band geometry, tab placement, and the pointer region.
//!
//! The design reserves a band at the top of the window, *inside* declared
//! geometry, and places one tab in it. Two properties matter enough to prove:
//!
//! 1. The pointer region covers the tab and nothing else. The rail this design
//!    replaces claimed the entire left window edge and swallowed the events,
//!    which is what blocked edge resize.
//! 2. Failure is typed. There is no "draw nothing" arm: either the chrome is
//!    drawn, or the caller is told why it cannot be, so an unlabelled window
//!    can never be the silent result of a layout edge case.

/// Minimum band height in logical px.
///
/// Also the vertical target size, so this cannot be lowered without failing
/// WCAG 2.2 SC 2.5.8.
pub const MIN_BAND_HEIGHT: u32 = 32;

/// Minimum interactive target, logical px (WCAG 2.2 SC 2.5.8).
pub const MIN_TARGET: u32 = 24;

/// Inset of the tab from the band's edges, logical px.
pub const TAB_INSET: u32 = 1;

/// Smallest window the chrome will decorate. Below this, the band would
/// dominate the window rather than annotate it.
pub const MIN_CONTENT_WIDTH: u32 = 120;
pub const MIN_CONTENT_HEIGHT: u32 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn right(&self) -> i32 {
        self.x + self.width as i32
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height as i32
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// What the caller asked to lay out.
#[derive(Debug, Clone, Copy)]
pub struct LayoutInput {
    /// Guest content size, logical px.
    pub content_width: u32,
    pub content_height: u32,
    /// Measured width of the tab's contents.
    pub tab_width: u32,
    /// Height needed by the tallest row of tab content.
    pub row_height: u32,
    /// Whether host-verified identity is available for this window.
    pub identity_verified: bool,
}

/// Why chrome could not be drawn.
///
/// Every variant is a refusal to decorate, never a decision to show guest
/// pixels unlabelled: the caller is expected to withhold the window or show a
/// proxy-owned placeholder, which is why the reason is carried rather than
/// discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailClosedReason {
    /// Identity could not be verified. Showing the window without a label
    /// would make it indistinguishable from an unproxied local window.
    IdentityUnverified,
    /// The window is too small for the band to be an annotation rather than a
    /// takeover.
    ContentTooSmall { width: u32, height: u32 },
    /// The tab cannot fit even at its minimum.
    TabDoesNotFit { needed: u32, available: u32 },
}

/// The result of laying out the chrome.
///
/// There is no third arm. A caller cannot accidentally handle "no chrome" as
/// "chrome that happens to be empty".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Decorate(Layout),
    FailClosed(FailClosedReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// The reserved strip, in wrapper coordinates.
    pub band: Rect,
    /// The tab within the band.
    pub tab: Rect,
    /// Where the guest surface is placed.
    pub guest_offset_y: i32,
    /// Size the wrapper reports to the compositor.
    pub outer_width: u32,
    pub outer_height: u32,
}

impl Layout {
    /// The pointer region, which must cover the tab and nothing else.
    pub fn input_region(&self) -> Rect {
        self.tab
    }
}

/// Resolve the layout, or refuse with a reason.
pub fn resolve(input: LayoutInput) -> Outcome {
    if !input.identity_verified {
        return Outcome::FailClosed(FailClosedReason::IdentityUnverified);
    }
    if input.content_width < MIN_CONTENT_WIDTH || input.content_height < MIN_CONTENT_HEIGHT {
        return Outcome::FailClosed(FailClosedReason::ContentTooSmall {
            width: input.content_width,
            height: input.content_height,
        });
    }

    // The band grows from the content it must hold, never the reverse: sizing
    // the band first and then fitting rows into it is what produced squashed
    // text in the design this replaces.
    let band_height = (input.row_height + TAB_INSET * 2).max(MIN_BAND_HEIGHT);
    let tab_height = band_height - TAB_INSET * 2;
    let tab_width = input.tab_width.max(MIN_TARGET);

    // Keep the tab clear of both window edges so it can never be mistaken for,
    // or interfere with, an edge-resize grab.
    let available = input.content_width.saturating_sub(TAB_INSET * 2 + 2);
    if tab_width > available {
        return Outcome::FailClosed(FailClosedReason::TabDoesNotFit {
            needed: tab_width,
            available,
        });
    }

    Outcome::Decorate(Layout {
        band: Rect {
            x: 0,
            y: 0,
            width: input.content_width,
            height: band_height,
        },
        tab: Rect {
            x: TAB_INSET as i32 + 1,
            y: TAB_INSET as i32,
            width: tab_width,
            height: tab_height,
        },
        guest_offset_y: band_height as i32,
        outer_width: input.content_width,
        outer_height: input.content_height + band_height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(width: u32, height: u32) -> LayoutInput {
        LayoutInput {
            content_width: width,
            content_height: height,
            tab_width: 80,
            row_height: 16,
            identity_verified: true,
        }
    }

    fn decorate(input: LayoutInput) -> Layout {
        match resolve(input) {
            Outcome::Decorate(l) => l,
            other => panic!("expected Decorate, got {other:?}"),
        }
    }

    #[test]
    fn the_band_is_reserved_at_the_top_and_pushes_the_guest_down() {
        let l = decorate(ok(800, 600));
        assert_eq!(l.band.y, 0);
        assert_eq!(l.outer_width, 800, "width must not change");
        assert_eq!(l.outer_height, 600 + l.band.height);
        assert_eq!(l.guest_offset_y, l.band.height as i32);
    }

    #[test]
    fn the_band_never_shrinks_below_the_target_size_floor() {
        for row_height in 0..MIN_BAND_HEIGHT {
            let mut input = ok(800, 600);
            input.row_height = row_height;
            assert!(decorate(input).band.height >= MIN_BAND_HEIGHT);
        }
    }

    #[test]
    fn the_band_grows_to_hold_taller_content_instead_of_squashing_it() {
        let mut input = ok(800, 600);
        input.row_height = 60;
        let l = decorate(input);
        assert!(
            l.band.height >= 60,
            "band {} cannot hold a 60px row",
            l.band.height
        );
        assert!(l.tab.height >= 60);
    }

    #[test]
    fn the_pointer_region_is_the_tab_and_nothing_else() {
        // The defect this design exists to remove: the rail's region covered
        // the whole left window edge and swallowed the events.
        let l = decorate(ok(800, 600));
        let r = l.input_region();
        assert_eq!(r, l.tab);
        assert!(r.x > 0, "region touches the left edge");
        assert!(r.right() < 800, "region touches the right edge");
        assert!(r.bottom() <= l.band.bottom(), "region escapes the band");
        assert!(
            r.height < 600,
            "region must not span the window height as the rail did"
        );
    }

    #[test]
    fn the_pointer_region_never_covers_guest_content() {
        let l = decorate(ok(800, 600));
        let r = l.input_region();
        // Guest content starts at guest_offset_y in wrapper coordinates.
        assert!(
            r.bottom() <= l.guest_offset_y,
            "region reaches into guest content"
        );
    }

    #[test]
    fn a_narrow_tab_is_widened_to_the_target_floor() {
        let mut input = ok(800, 600);
        input.tab_width = 4;
        assert_eq!(decorate(input).tab.width, MIN_TARGET);
    }

    #[test]
    fn unverified_identity_fails_closed_with_a_reason() {
        let mut input = ok(800, 600);
        input.identity_verified = false;
        assert_eq!(
            resolve(input),
            Outcome::FailClosed(FailClosedReason::IdentityUnverified)
        );
    }

    #[test]
    fn a_tiny_window_fails_closed_rather_than_being_swallowed_by_its_band() {
        assert!(matches!(
            resolve(ok(40, 20)),
            Outcome::FailClosed(FailClosedReason::ContentTooSmall { .. })
        ));
    }

    #[test]
    fn a_tab_that_cannot_fit_fails_closed_with_the_numbers() {
        let mut input = ok(200, 600);
        input.tab_width = 400;
        match resolve(input) {
            Outcome::FailClosed(FailClosedReason::TabDoesNotFit { needed, available }) => {
                assert_eq!(needed, 400);
                assert!(available < needed);
            }
            other => panic!("expected TabDoesNotFit, got {other:?}"),
        }
    }

    #[test]
    fn there_is_no_undecorated_success_arm() {
        // Enforced by the type: Outcome has exactly two variants, so a caller
        // cannot fall through to showing guest pixels unlabelled. This test
        // exists so that adding a third variant breaks something loudly.
        let l = decorate(ok(800, 600));
        match resolve(ok(800, 600)) {
            Outcome::Decorate(d) => assert_eq!(d, l),
            Outcome::FailClosed(_) => panic!("unexpected"),
        }
    }

    #[test]
    fn layout_is_deterministic() {
        for _ in 0..8 {
            assert_eq!(decorate(ok(1280, 720)), decorate(ok(1280, 720)));
        }
    }

    #[test]
    fn reserving_the_band_is_the_only_change_to_reported_size() {
        // The compositor lays out and borders the band-inclusive rect, so the
        // relationship between what the guest asked for and what the wrapper
        // reports has to be exactly one added band and nothing else.
        for (w, h) in [(320_u32, 200_u32), (800, 600), (1920, 1080), (3840, 2160)] {
            let l = decorate(ok(w, h));
            assert_eq!(l.outer_width, w);
            assert_eq!(l.outer_height - h, l.band.height);
        }
    }
}
