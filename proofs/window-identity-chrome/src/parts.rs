//! The parts model: an ordered, measured list of tab contents.
//!
//! This is the customization contract, and it is also the reason the tab's
//! pointer handling is correct.
//!
//! The tab used to be laid out by one arithmetic expression and hit-tested by a
//! second, independent one. Those two expressions have to agree about padding,
//! tracking, icon pitch and separator width, forever, or the user clicks one
//! control and activates another. During prototyping they disagreed three
//! separate times, which is what "clicking doesn't select the item I clicked
//! on" actually was.
//!
//! So there is exactly one list. [`Parts::layout`] measures every part once and
//! records the box it occupies. Drawing walks that list. Hit-testing walks the
//! same list. A part's hit box is the box it was drawn into, by construction,
//! and the two cannot drift apart because there is nothing to keep in sync.
//!
//! Customization then falls out for free: a part is a value in a vector, so
//! reordering, adding and removing parts is a config edit rather than an edit
//! to the layout arithmetic.

use serde::{Deserialize, Serialize};

use crate::action::Action;
use crate::measure::Measure;

/// WCAG 2.2 SC 2.5.8 target-size minimum, in logical px.
///
/// Every interactive part is widened to at least this, so a narrow glyph can
/// never produce a target the user cannot reliably hit. The band already
/// guarantees the vertical dimension.
pub const MIN_TARGET_PX: f32 = 24.0;

/// Upper bound on a configured spacer, so a config cannot push the tab across
/// the window by accident.
pub const MAX_SPACER_PX: u16 = 64;

/// Upper bound on parts per row. A row longer than this is a configuration
/// mistake, not a preference, and would push the tab past most windows.
pub const MAX_PARTS_PER_ROW: usize = 16;

/// One renderable element of the tab.
///
/// Serialized as a flat string (`"identity"`, `"audio"`, `"spacer:8"`) rather
/// than as a tagged enum. A generated config is something an operator reads and
/// diffs, so `["identity", "chevron", "separator", "audio"]` is the right wire
/// form; serde's default for a newtype variant would produce
/// `{"action": "audio"}` in the middle of that list, which is noise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    /// The workload name. Carries identity, so it is mandatory: see
    /// [`PartsConfig::validate`].
    Identity,
    /// The expand/collapse affordance. Mandatory whenever an expanded row
    /// exists, otherwise the row is unreachable by pointer.
    Chevron,
    /// A hairline rule with symmetric spacing.
    Separator,
    /// The security-capability token (`UNVERIFIED`, `MIC`, `USB`, ...).
    Status,
    /// An action icon button.
    Action(Action),
    /// Fixed inert space, in logical px.
    Spacer(u16),
}

impl std::fmt::Display for Part {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Part::Spacer(px) => write!(f, "spacer:{px}"),
            other => f.write_str(other.name()),
        }
    }
}

impl std::str::FromStr for Part {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(px) = s.strip_prefix("spacer:") {
            let px: u16 = px
                .parse()
                .map_err(|_| format!("`{s}`: spacer width must be a whole number of px"))?;
            if px > MAX_SPACER_PX {
                return Err(format!(
                    "`{s}`: spacer width must be at most {MAX_SPACER_PX} px"
                ));
            }
            return Ok(Part::Spacer(px));
        }
        Ok(match s {
            "identity" => Part::Identity,
            "chevron" => Part::Chevron,
            "separator" => Part::Separator,
            "status" => Part::Status,
            "spacer" => Part::Spacer(4),
            "open-terminal" => Part::Action(Action::Terminal),
            "audio-controls" => Part::Action(Action::Audio),
            "usb-devices" => Part::Action(Action::Usb),
            "vm-details" => Part::Action(Action::Info),
            "stop-vm" => Part::Action(Action::Stop),
            other => {
                return Err(format!(
                    "unknown part `{other}`; expected one of identity, chevron, separator, \
                     status, spacer, spacer:<px>, open-terminal, audio-controls, usb-devices, \
                     vm-details, stop-vm"
                ))
            }
        })
    }
}

impl Serialize for Part {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Part {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl Part {
    /// What a press on this part means. Inert parts never take a press, so
    /// they cannot swallow a click that was aimed past them.
    pub fn hit_kind(&self) -> HitKind {
        match self {
            Part::Identity | Part::Chevron => HitKind::Toggle,
            Part::Status => HitKind::Status,
            Part::Action(a) => HitKind::Action(*a),
            Part::Separator | Part::Spacer(_) => HitKind::Inert,
        }
    }

    /// Stable name for config, logging and tests.
    pub fn name(&self) -> &'static str {
        match self {
            Part::Identity => "identity",
            Part::Chevron => "chevron",
            Part::Separator => "separator",
            Part::Status => "status",
            Part::Spacer(_) => "spacer",
            Part::Action(a) => a.name(),
        }
    }
}

/// The meaning of a press, resolved from the part that was hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitKind {
    /// Expand or collapse.
    Toggle,
    /// Open the detail surface.
    Status,
    /// Invoke an action.
    Action(Action),
    /// Consumes no press.
    Inert,
}

/// A part, measured and placed. `x` and `width` are physical px.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub part: Part,
    pub x: f32,
    pub width: f32,
    /// Advance width of the drawn glyphs, for parts that draw text. Drawing
    /// needs this to position the chevron directly after the label rather than
    /// after the label's padded box.
    pub text_advance: f32,
}

impl Placed {
    pub fn end(&self) -> f32 {
        self.x + self.width
    }

    pub fn contains(&self, x: f32) -> bool {
        x >= self.x && x < self.end()
    }
}

/// Metrics needed to measure parts. Physical px unless stated.
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    pub scale: f32,
    pub font_px: f32,
    pub tracking: f32,
    /// Leading furniture: the accent bar plus its breathing room.
    pub left_furniture: f32,
    pub side_pad: f32,
    pub chevron_width: f32,
    pub chevron_gap: f32,
    pub icon_box: f32,
    pub icon_gap: f32,
    pub sep_gap: f32,
    pub sep_width: f32,
    /// Draw a text label beside each action icon.
    ///
    /// On by default. Five bare glyphs are not self-describing, and the ones
    /// that open further controls (audio, USB, details) cannot be guessed from
    /// a glyph at all.
    pub action_labels: bool,
    /// Gap between an action's icon and its label.
    pub label_gap: f32,
}

impl Metrics {
    /// The target-size floor, in physical px.
    pub fn min_target(&self) -> f32 {
        MIN_TARGET_PX * self.scale
    }
}

/// The measured tab: parts in order, with the box each one owns.
#[derive(Debug, Clone, PartialEq)]
pub struct Parts {
    pub placed: Vec<Placed>,
    pub width: f32,
}

impl Parts {
    /// Measure `parts` starting at x = 0.
    ///
    /// Padding is applied *between* parts and at the two ends, so the visible
    /// gap on the left of the first part equals the gap on the right of the
    /// last one. Measuring from the accent bar rather than from the tab edge is
    /// what makes the label look optically centred.
    pub fn layout(parts: &[Part], m: &Metrics, font: &dyn Measure, label: &str) -> Parts {
        let mut placed: Vec<Placed> = Vec::with_capacity(parts.len());
        let mut x = m.left_furniture;
        let mut first = true;

        for part in parts {
            let (mut width, advance) = match part {
                Part::Identity => {
                    let a = font.measure(label, m.font_px, m.tracking);
                    (a, a)
                }
                Part::Status => {
                    let a = font.measure("STATUS", m.font_px * 0.85, m.tracking);
                    (a, a)
                }
                Part::Chevron => (m.chevron_width, 0.0),
                Part::Separator => (m.sep_width.max(1.0), 0.0),
                Part::Action(a) => {
                    if m.action_labels {
                        let text = font.measure(a.label(), m.font_px, m.tracking);
                        (m.icon_box + m.label_gap + text, text)
                    } else {
                        (m.icon_box, 0.0)
                    }
                }
                Part::Spacer(px) => (f32::from(*px) * m.scale, 0.0),
            };

            // WCAG 2.2 SC 2.5.8: no interactive part may be narrower than the
            // target floor. Widening here rather than at draw time keeps the
            // hit box and the drawn box identical.
            if part.hit_kind() != HitKind::Inert {
                width = width.max(m.min_target());
            }

            // Space before this part.
            let lead = if first {
                m.side_pad
            } else {
                match part {
                    Part::Chevron => m.chevron_gap,
                    Part::Separator => m.sep_gap,
                    Part::Action(_) => match placed.last().map(|p| &p.part) {
                        Some(Part::Action(_)) => m.icon_gap,
                        Some(Part::Separator) => m.sep_gap,
                        _ => m.side_pad,
                    },
                    Part::Spacer(_) => 0.0,
                    _ => m.side_pad,
                }
            };
            // A separator's own leading gap is symmetric, so the part after it
            // does not need to re-add one; that is handled above.
            x += lead;
            first = false;

            placed.push(Placed {
                part: part.clone(),
                x,
                width,
                text_advance: advance,
            });
            x += width;
        }

        // Trailing padding mirrors the leading padding, measured from the same
        // reference, so `Work >` has equal space either side.
        let width = x + m.side_pad;
        Parts { placed, width }
    }

    /// Find the part under `x` (physical px, tab-relative).
    ///
    /// Every x inside the tab resolves to a part. The inter-part padding is not
    /// dead zone: each interactive part claims outward to the midpoint of the
    /// gap between it and its interactive neighbour, and the first and last
    /// claim out to the tab's edges. A press that lands on the accent bar, or
    /// in the padding beside an icon, therefore activates the control the user
    /// was plainly aiming at, instead of being silently swallowed.
    ///
    /// Inert parts are skipped entirely, so a separator between two icons is
    /// split between them rather than absorbing presses.
    pub fn hit(&self, x: f32) -> Option<&Placed> {
        // The upper bound is inclusive. Callers bound-check against the ceiled
        // pointer region, which can be up to a pixel wider than the measured
        // tab, and the last part legitimately claims out to the tab's edge.
        // Excluding `width` here would leave that sliver resolving to nothing.
        if x < 0.0 || x > self.width {
            return None;
        }
        let live: Vec<&Placed> = self
            .placed
            .iter()
            .filter(|p| p.part.hit_kind() != HitKind::Inert)
            .collect();
        let (&first, &last) = (live.first()?, live.last()?);

        for (i, p) in live.iter().enumerate() {
            let lo = if i == 0 {
                0.0
            } else {
                (live[i - 1].end() + p.x) / 2.0
            };
            let hi = if i + 1 == live.len() {
                self.width
            } else {
                (p.end() + live[i + 1].x) / 2.0
            };
            if x >= lo && x < hi {
                return Some(p);
            }
        }
        // Unreachable for a well-formed layout; prefer the nearer end over a
        // press that does nothing.
        Some(if x < first.x { first } else { last })
    }

    pub fn find(&self, name: &str) -> Option<&Placed> {
        self.placed.iter().find(|p| p.part.name() == name)
    }
}

/// The user-facing configuration: which parts appear, collapsed and expanded.
///
/// This mirrors waybar's `modules-*` shape, but it is a *generated* artifact.
/// d2b's convention is that Nix is the sole authority and emits JSON that the
/// Rust side deserializes with serde, exactly like every other bundle artifact,
/// so this type is written to be liftable into `d2b-core` unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct PartsConfig {
    /// Shown at rest.
    pub collapsed: Vec<Part>,
    /// Shown once expanded. Must preserve `collapsed` as a prefix, so identity
    /// never moves when the tab opens.
    pub expanded: Vec<Part>,
    /// Draw action icons without their labels.
    ///
    /// Off by default: labels are what make the actions self-describing, and
    /// three independent accessibility reviews rejected bare glyphs. This exists
    /// for operators who have learned the icons and want the smaller footprint,
    /// and is recorded as a deliberate accessibility trade-off.
    #[serde(default)]
    pub compact_actions: bool,
}

impl Default for PartsConfig {
    fn default() -> Self {
        Self {
            collapsed: vec![Part::Identity, Part::Chevron],
            expanded: vec![
                Part::Identity,
                Part::Chevron,
                Part::Separator,
                Part::Action(Action::Terminal),
                Part::Action(Action::Audio),
                Part::Action(Action::Usb),
                Part::Action(Action::Info),
                Part::Action(Action::Stop),
            ],
            compact_actions: false,
        }
    }
}

/// Why a configuration was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// Identity is what the whole surface exists to communicate. A config that
    /// removes it is refused rather than honoured, because honouring it would
    /// produce an unlabelled window, which is the security failure this design
    /// is meant to prevent.
    MissingIdentity { row: &'static str },
    /// Two identity parts make "which VM is this" ambiguous, which is worse
    /// than none at all.
    RepeatedIdentity { row: &'static str },
    /// An expanded row that cannot be reached is a trap.
    UnreachableExpansion,
    /// Identity must not move when the tab opens. Requiring `expanded` to
    /// extend `collapsed` makes that a property of the config rather than
    /// something the renderer has to preserve by convention.
    ExpansionReordersRestingParts { index: usize },
    /// Duplicates make hit-testing ambiguous to the user even though the code
    /// resolves them deterministically.
    DuplicatePart { name: String, row: &'static str },
    Empty { row: &'static str },
    TooManyParts { row: &'static str, count: usize },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::MissingIdentity { row } => write!(
                f,
                "the `{row}` row must contain the `identity` part: identity is not optional"
            ),
            ConfigError::RepeatedIdentity { row } => write!(
                f,
                "the `{row}` row lists `identity` more than once: a window has one identity"
            ),
            ConfigError::UnreachableExpansion => write!(
                f,
                "the `collapsed` row must contain `chevron` when `expanded` differs from it, \
                 otherwise the expanded row can never be opened"
            ),
            ConfigError::ExpansionReordersRestingParts { index } => write!(
                f,
                "the `expanded` row must begin with the whole `collapsed` row, in order; \
                 position {index} differs. Expansion may only append, so identity does not \
                 move when the tab opens"
            ),
            ConfigError::DuplicatePart { name, row } => {
                write!(f, "the `{row}` row lists `{name}` more than once")
            }
            ConfigError::Empty { row } => write!(f, "the `{row}` row is empty"),
            ConfigError::TooManyParts { row, count } => write!(
                f,
                "the `{row}` row has {count} parts; at most {MAX_PARTS_PER_ROW} are allowed"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

impl PartsConfig {
    /// Reject configurations that would produce an unusable, unlabelled, or
    /// self-contradicting tab.
    ///
    /// This fails closed at config-load time. The alternative -- silently
    /// substituting a default -- would leave the operator believing their
    /// config applied.
    pub fn validate(&self) -> Result<(), ConfigError> {
        for (row, parts) in [("collapsed", &self.collapsed), ("expanded", &self.expanded)] {
            if parts.is_empty() {
                return Err(ConfigError::Empty { row });
            }
            if parts.len() > MAX_PARTS_PER_ROW {
                return Err(ConfigError::TooManyParts {
                    row,
                    count: parts.len(),
                });
            }
            match parts.iter().filter(|p| **p == Part::Identity).count() {
                0 => return Err(ConfigError::MissingIdentity { row }),
                1 => {}
                _ => return Err(ConfigError::RepeatedIdentity { row }),
            }
            let mut seen: Vec<&str> = Vec::new();
            for p in parts.iter() {
                // Inert parts are pure spacing, so repeating them is a layout
                // choice rather than an ambiguity.
                if p.hit_kind() == HitKind::Inert {
                    continue;
                }
                if seen.contains(&p.name()) {
                    return Err(ConfigError::DuplicatePart {
                        name: p.name().to_string(),
                        row,
                    });
                }
                seen.push(p.name());
            }
        }
        if self.expanded != self.collapsed {
            if !self.collapsed.contains(&Part::Chevron) {
                return Err(ConfigError::UnreachableExpansion);
            }
            for (i, resting) in self.collapsed.iter().enumerate() {
                if self.expanded.get(i) != Some(resting) {
                    return Err(ConfigError::ExpansionReordersRestingParts { index: i });
                }
            }
        }
        Ok(())
    }

    /// Parse and validate. Parsing alone is not enough: an unvalidated config
    /// is how an unlabelled tab would reach a screen.
    pub fn from_json(s: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let cfg: PartsConfig = serde_json::from_str(s)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn row(&self, expanded: bool) -> &[Part] {
        if expanded {
            &self.expanded
        } else {
            &self.collapsed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure::FixedMetric;


    fn font() -> FixedMetric {
        FixedMetric
    }

    fn metrics() -> Metrics {
        Metrics {
            scale: 1.0,
            font_px: 12.0,
            tracking: 0.0,
            left_furniture: 4.0,
            side_pad: 8.0,
            chevron_width: 9.0,
            chevron_gap: 5.0,
            icon_box: 18.0,
            icon_gap: 4.0,
            sep_gap: 6.0,
            sep_width: 1.0,
            action_labels: true,
            label_gap: 5.0,
        }
    }

    #[test]
    fn leading_and_trailing_padding_are_equal() {
        let m = metrics();
        let p = Parts::layout(&PartsConfig::default().collapsed, &m, &font(), "Work");
        let first = &p.placed[0];
        let last = p.placed.last().unwrap();
        let lead = first.x - m.left_furniture;
        let trail = p.width - last.end();
        assert!(
            (lead - trail).abs() < 0.01,
            "asymmetric padding: lead {lead} trail {trail}"
        );
    }

    #[test]
    fn every_part_is_hit_by_its_own_centre() {
        // The property that the old two-expression layout kept violating.
        let m = metrics();
        let cfg = PartsConfig::default();
        let p = Parts::layout(&cfg.expanded, &m, &font(), "Work");
        for placed in &p.placed {
            if placed.part.hit_kind() == HitKind::Inert {
                continue;
            }
            let centre = placed.x + placed.width / 2.0;
            let hit = p.hit(centre).expect("centre of a part must hit something");
            assert_eq!(
                hit.part, placed.part,
                "centre of {} hit {} instead",
                placed.part.name(),
                hit.part.name()
            );
        }
    }

    #[test]
    fn no_part_boxes_overlap() {
        let m = metrics();
        let cfg = PartsConfig::default();
        let p = Parts::layout(&cfg.expanded, &m, &font(), "corp-workstation.work");
        for w in p.placed.windows(2) {
            assert!(
                w[0].end() <= w[1].x + 0.001,
                "{} overlaps {}",
                w[0].part.name(),
                w[1].part.name()
            );
        }
    }

    #[test]
    fn parts_stay_inside_the_measured_width() {
        let m = metrics();
        let cfg = PartsConfig::default();
        let p = Parts::layout(&cfg.expanded, &m, &font(), "Work");
        for placed in &p.placed {
            assert!(placed.x >= 0.0);
            assert!(
                placed.end() <= p.width,
                "{} runs past the tab",
                placed.part.name()
            );
        }
    }

    #[test]
    fn every_x_inside_the_tab_resolves_to_a_part() {
        // No dead zones: a press anywhere on the tab does something, because a
        // press that silently does nothing reads as an unresponsive control.
        let m = metrics();
        let cfg = PartsConfig::default();
        let p = Parts::layout(&cfg.expanded, &m, &font(), "Work");
        let mut x = 0.0f32;
        while x < p.width {
            assert!(p.hit(x).is_some(), "dead zone at x={x}");
            x += 0.5;
        }
    }

    #[test]
    fn hit_outside_the_tab_is_refused_beyond_the_slack() {
        let m = metrics();
        let cfg = PartsConfig::default();
        let p = Parts::layout(&cfg.collapsed, &m, &font(), "Work");
        assert!(p.hit(p.width + 40.0).is_none());
        assert!(p.hit(-40.0).is_none());
    }

    #[test]
    fn scaling_keeps_ordering_and_proportions() {
        let font = font();
        let mut m1 = metrics();
        m1.scale = 1.0;
        let a = Parts::layout(&PartsConfig::default().expanded, &m1, &font, "Work");

        let mut m2 = metrics();
        m2.scale = 2.0;
        m2.font_px = 24.0;
        m2.left_furniture = 8.0;
        m2.side_pad = 16.0;
        m2.chevron_width = 18.0;
        m2.chevron_gap = 10.0;
        m2.icon_box = 36.0;
        m2.icon_gap = 8.0;
        m2.sep_gap = 12.0;
        m2.sep_width = 2.0;
        m2.label_gap = 10.0;
        let b = Parts::layout(&PartsConfig::default().expanded, &m2, &font, "Work");

        assert_eq!(a.placed.len(), b.placed.len());
        assert!(
            (b.width / a.width - 2.0).abs() < 0.02,
            "2x metrics should give ~2x width, got {} vs {}",
            b.width,
            a.width
        );
    }

    #[test]
    fn default_config_validates() {
        PartsConfig::default().validate().unwrap();
    }

    #[test]
    fn config_without_identity_is_refused() {
        let cfg = PartsConfig {
            collapsed: vec![Part::Chevron],
            expanded: vec![Part::Chevron],
            compact_actions: false,
        };
        assert_eq!(
            cfg.validate(),
            Err(ConfigError::MissingIdentity { row: "collapsed" })
        );
    }

    #[test]
    fn expansion_without_a_chevron_is_refused() {
        let cfg = PartsConfig {
            collapsed: vec![Part::Identity],
            expanded: vec![Part::Identity, Part::Action(Action::Stop)],
            compact_actions: false,
        };
        assert_eq!(cfg.validate(), Err(ConfigError::UnreachableExpansion));
    }

    #[test]
    fn duplicate_parts_are_refused_but_spacers_repeat() {
        let dup = PartsConfig {
            collapsed: vec![Part::Identity, Part::Chevron, Part::Chevron],
            expanded: vec![Part::Identity, Part::Chevron],
            compact_actions: false,
        };
        assert!(matches!(
            dup.validate(),
            Err(ConfigError::DuplicatePart { .. })
        ));

        let spacers = PartsConfig {
            collapsed: vec![
                Part::Identity,
                Part::Spacer(4),
                Part::Spacer(4),
                Part::Chevron,
            ],
            expanded: vec![
                Part::Identity,
                Part::Spacer(4),
                Part::Spacer(4),
                Part::Chevron,
            ],
            compact_actions: false,
        };
        spacers.validate().unwrap();
    }

    #[test]
    fn json_round_trips_and_reads_naturally() {
        let cfg = PartsConfig::default();
        let json = serde_json::to_string_pretty(&cfg).unwrap();
        // Actions must appear as bare names, not as tagged objects: this is a
        // file an operator reads and diffs.
        assert!(json.contains("\"identity\""), "{json}");
        assert!(json.contains("\"chevron\""), "{json}");
        assert!(json.contains("\"open-terminal\""), "{json}");
        assert!(
            !json.contains("{\"action\""),
            "actions must not be tagged: {json}"
        );
        let back = PartsConfig::from_json(&json).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn a_hand_written_config_reads_the_way_an_operator_would_write_it() {
        let json = r#"{
          "collapsed": ["identity", "chevron"],
          "expanded": ["identity", "chevron", "separator", "audio-controls", "usb-devices", "open-terminal"]
        }"#;
        let cfg = PartsConfig::from_json(json).expect("flat part names must parse");
        assert_eq!(
            cfg.expanded,
            vec![
                Part::Identity,
                Part::Chevron,
                Part::Separator,
                Part::Action(Action::Audio),
                Part::Action(Action::Usb),
                Part::Action(Action::Terminal),
            ]
        );
    }

    #[test]
    fn spacers_carry_their_width_through_the_wire_form() {
        let cfg = PartsConfig {
            collapsed: vec![Part::Identity, Part::Spacer(12), Part::Chevron],
            expanded: vec![Part::Identity, Part::Spacer(12), Part::Chevron],
            compact_actions: false,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("\"spacer:12\""), "{json}");
        assert_eq!(PartsConfig::from_json(&json).unwrap(), cfg);
    }

    #[test]
    fn an_unknown_part_names_the_valid_ones() {
        let err = "hologram".parse::<Part>().unwrap_err();
        assert!(err.contains("identity"), "{err}");
        assert!(err.contains("audio-controls"), "{err}");
    }

    #[test]
    fn unknown_parts_are_refused_rather_than_ignored() {
        let json = r#"{"collapsed":["identity","hologram"],"expanded":["identity"]}"#;
        assert!(PartsConfig::from_json(json).is_err());
    }

    #[test]
    fn unknown_config_keys_are_refused() {
        let json = r#"{"collapsed":["identity"],"expanded":["identity"],"colour":"red"}"#;
        assert!(PartsConfig::from_json(json).is_err());
    }

    #[test]
    fn reordering_parts_moves_their_hit_boxes_with_them() {
        // The point of the parts model: custom order needs no layout change.
        let m = metrics();
        let font = font();
        let reordered = vec![
            Part::Action(Action::Stop),
            Part::Separator,
            Part::Identity,
            Part::Chevron,
        ];
        let p = Parts::layout(&reordered, &m, &font, "Work");
        let stop = p.find("stop-vm").unwrap();
        let identity = p.find("identity").unwrap();
        assert!(stop.x < identity.x, "stop should now precede identity");
        let hit = p.hit(stop.x + stop.width / 2.0).unwrap();
        assert_eq!(hit.part.hit_kind(), HitKind::Action(Action::Stop));
    }

    #[test]
    fn inert_parts_never_claim_a_press() {
        let m = metrics();
        let p = Parts::layout(
            &[Part::Identity, Part::Separator, Part::Chevron],
            &m,
            &font(),
            "Work",
        );
        let sep = p.find("separator").unwrap();
        let hit = p.hit(sep.x + sep.width / 2.0).unwrap();
        assert_ne!(hit.part, Part::Separator);
    }

    #[test]
    fn a_long_label_grows_the_tab_without_disturbing_order() {
        let m = metrics();
        let font = font();
        let cfg = PartsConfig::default();
        let short = Parts::layout(&cfg.expanded, &m, &font, "Work");
        let long = Parts::layout(&cfg.expanded, &m, &font, "corp-workstation.work");
        assert!(long.width > short.width);
        let names: Vec<_> = short.placed.iter().map(|p| p.part.name()).collect();
        let long_names: Vec<_> = long.placed.iter().map(|p| p.part.name()).collect();
        assert_eq!(names, long_names);
    }
}

#[cfg(test)]
mod a11y_tests {
    use super::*;
    use crate::measure::FixedMetric;


    fn font() -> FixedMetric {
        FixedMetric
    }

    fn metrics_at(scale: f32, labels: bool) -> Metrics {
        Metrics {
            scale,
            font_px: 12.0 * scale,
            tracking: 0.0,
            left_furniture: 7.0 * scale,
            side_pad: 8.0 * scale,
            chevron_width: 9.0 * scale,
            chevron_gap: 5.0 * scale,
            icon_box: 18.0 * scale,
            icon_gap: 4.0 * scale,
            sep_gap: 6.0 * scale,
            sep_width: 1.0 * scale,
            action_labels: labels,
            label_gap: 5.0 * scale,
        }
    }

    #[test]
    fn every_interactive_part_meets_the_target_size_floor_at_every_scale() {
        // WCAG 2.2 SC 2.5.8. The band guarantees the vertical dimension; this
        // is the horizontal one, which a narrow glyph would otherwise fail.
        let font = font();
        for scale in [1.0_f32, 1.25, 1.5, 2.0] {
            for labels in [true, false] {
                let m = metrics_at(scale, labels);
                let cfg = PartsConfig::default();
                for row in [&cfg.collapsed, &cfg.expanded] {
                    let p = Parts::layout(row, &m, &font, "Work");
                    for placed in &p.placed {
                        if placed.part.hit_kind() == HitKind::Inert {
                            continue;
                        }
                        let logical = placed.width / scale;
                        assert!(
                            logical >= MIN_TARGET_PX - 0.01,
                            "{} is {logical:.2} logical px at scale {scale} (labels={labels})",
                            placed.part.name()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn labels_are_on_by_default_and_widen_the_actions() {
        let font = font();
        let cfg = PartsConfig::default();
        assert!(!cfg.compact_actions, "labels must be the default");

        let labelled = Parts::layout(&cfg.expanded, &metrics_at(1.0, true), &font, "Work");
        let compact = Parts::layout(&cfg.expanded, &metrics_at(1.0, false), &font, "Work");
        assert!(
            labelled.width > compact.width,
            "labelled {} should exceed compact {}",
            labelled.width,
            compact.width
        );

        // And the labelled variant must still hit cleanly.
        let stop = labelled.find("stop-vm").unwrap();
        assert_eq!(
            labelled.hit(stop.x + stop.width / 2.0).unwrap().part,
            Part::Action(Action::Stop)
        );
    }

    #[test]
    fn identity_must_appear_exactly_once() {
        let cfg = PartsConfig {
            collapsed: vec![Part::Identity, Part::Identity, Part::Chevron],
            expanded: vec![Part::Identity, Part::Chevron],
            compact_actions: false,
        };
        assert_eq!(
            cfg.validate(),
            Err(ConfigError::RepeatedIdentity { row: "collapsed" })
        );
    }

    #[test]
    fn expansion_may_only_append_so_identity_never_moves() {
        // Reordering on expansion would make the label jump under the pointer
        // at the exact moment the user is aiming at it.
        let cfg = PartsConfig {
            collapsed: vec![Part::Identity, Part::Chevron],
            expanded: vec![
                Part::Chevron,
                Part::Identity,
                Part::Action(Action::Stop),
            ],
            compact_actions: false,
        };
        assert_eq!(
            cfg.validate(),
            Err(ConfigError::ExpansionReordersRestingParts { index: 0 })
        );
    }

    #[test]
    fn identity_keeps_its_position_when_the_tab_expands() {
        let font = font();
        let m = metrics_at(1.0, true);
        let cfg = PartsConfig::default();
        let collapsed = Parts::layout(&cfg.collapsed, &m, &font, "Work");
        let expanded = Parts::layout(&cfg.expanded, &m, &font, "Work");
        assert_eq!(
            collapsed.find("identity").unwrap().x,
            expanded.find("identity").unwrap().x,
            "identity must not move when the tab opens"
        );
    }

    #[test]
    fn an_overlong_row_is_refused() {
        let mut collapsed = vec![Part::Identity, Part::Chevron];
        collapsed.extend(std::iter::repeat_n(Part::Spacer(1), MAX_PARTS_PER_ROW));
        let cfg = PartsConfig {
            expanded: collapsed.clone(),
            collapsed,
            compact_actions: false,
        };
        assert!(matches!(
            cfg.validate(),
            Err(ConfigError::TooManyParts { .. })
        ));
    }

    #[test]
    fn an_oversized_spacer_is_refused_with_its_bound() {
        let err = format!("spacer:{}", MAX_SPACER_PX as u32 + 1)
            .parse::<Part>()
            .unwrap_err();
        assert!(err.contains(&MAX_SPACER_PX.to_string()), "{err}");
    }

    #[test]
    fn destructive_and_submenu_actions_are_declared() {
        // The renderer and the dispatcher both need to know that `stop-vm`
        // cannot be a one-release activation, and that audio/usb/details open
        // further controls rather than toggling something.
        assert!(Action::Stop.is_destructive());
        assert!(!Action::Terminal.is_destructive());
        for a in [Action::Audio, Action::Usb, Action::Info] {
            assert!(a.opens_submenu(), "{} must open a submenu", a.name());
        }
        assert!(!Action::Terminal.opens_submenu());
    }

    #[test]
    fn action_names_describe_outcomes() {
        for a in Action::DEFAULTS {
            let n = a.name();
            assert!(!n.is_empty());
            assert!(
                n.contains('-'),
                "`{n}` should read as an outcome, e.g. `open-terminal`"
            );
            assert!(!a.label().is_empty(), "{n} needs a visible label");
        }
    }
}
