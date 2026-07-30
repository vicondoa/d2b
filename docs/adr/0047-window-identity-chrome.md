# ADR 0047: Window identity chrome

- Status: Proposed
- Date: 2026-07-29
- Supersedes: the `d2b-wayland-proxy` wrapper rail (`WRAPPER_RAIL_WIDTH = 9`)
- Related: [ADR 0044](0044-unsafe-local-runtime-provider.md)
  (unsafe-local runtime provider, which introduced the Wayland-proxy identity
  rail), [ADR 0015](0015-daemon-only-clean-break.md) (daemon-only control
  plane, which owns action dispatch)
- Proof: [`proofs/window-identity-chrome/`](../../proofs/window-identity-chrome/)
- Prototype: `labs/window-chrome/`

## Context

`d2b-wayland-proxy` wraps every proxied guest toplevel in a proxy-owned wrapper
`xdg_toplevel` and paints a 9 px full-window-height rail down its left edge.
The rail exists to answer one question - *which VM does this window belong to?*
- and it answers it badly.

Read from the code, the defects are:

1. **The label is distorted.** It is drawn with a 5×7 bitmap font, rotated 90°,
   and stretched non-uniformly (`VERTICAL_LABEL_X_SCALE = 1`,
   `VERTICAL_LABEL_Y_SCALE = 2`) to fit 9 px. Rotated, non-uniformly scaled
   text is an accessibility failure as well as an ugly one.
2. **It fights the compositor.** Running the full window height puts it in
   direct collision with niri's rounded `geometry-corner-radius`, its border,
   and its gap language.
3. **It steals input.** The wrapper's input region is set to
   `(0, 0, rail_width, outer.height)` and `filter.rs` swallows every event in
   it. That covers the entire left window edge, including where a user aims for
   an edge-resize grab.
4. **It is a dead end.** It is visual-only by design, so there is nowhere to
   hang the per-VM actions operators actually want.

A 9 px hit target also fails WCAG 2.2 SC 2.5.8, and the rail's automatic text
colour uses a weighted-sum brightness threshold rather than a contrast ratio -
see [Colour selection](#colour-selection) for how badly that behaves.

### What the compositor allows

niri is the target compositor, and three of its properties are load-bearing.
All were read from niri's rendering source and then confirmed empirically in a
nested and then a live session.

- **Client subsurfaces render above niri's border.** `tile.rs`'s render path
  pushes window content before the border, so a surface drawn outside window
  geometry paints *over* the border. The intuitive request - "give the window a
  tab and let niri's border wrap around it" - is therefore not implementable.
  This eliminated every placement outside declared geometry.
- **niri does not advertise `zxdg_decoration_manager_v1`.** It is absent from
  all 41 globals. There is consequently no negotiation to perform, and the
  `draw-border-with-background false` window rule is the mechanism rather than
  a workaround.
- **`zwlr_layer_shell_v1` anchors to screen edges only.** A layer surface cannot
  track a window, which eliminates an anchored companion panel that follows the
  window.

Conversely, because the chrome lives *inside* declared geometry,
`clip-to-geometry true` **improves** it - the compositor rounds its corners -
rather than clipping it away.

## Decision

Replace the rail with a **reserved band at the top of the window, inside
declared geometry, containing one tab**.

### Geometry

- The band is reserved at the top of the wrapper. Window width is unchanged;
  the guest surface is offset downward by the band height.
- The band is **32 logical px minimum**, and **grows from the height its
  content needs**. Sizing the band first and fitting rows into it afterwards is
  what produced squashed text in the design being replaced.
- The tab is inset from the band's edges and never touches the window edges, so
  edge-resize grabs are unobstructed.
- The compositor lays out and borders the band-inclusive rect, so reserving the
  band is the *only* change to the size the wrapper reports.

### Appearance

The tab is drawn as **three concentric shapes**: an outer border-coloured
shape, a card inset uniformly from it, and an accent bar formed from the card's
own left columns clipped to the card. Concentricity is what makes the outer and
inner curves agree; deriving each shape independently does not.

- Fill is a neutral off-gray (`#25272b`), matching `d2b-wlcontrol`'s chip
  colour, so the chrome reads as part of the d2b desktop rather than as a
  coloured band.
- The realm accent appears as the left bar plus a hairline elsewhere.
- The label is **horizontal**, optically centred on cap height. No rotation, no
  non-uniform scaling.
- A disclosure mark follows the label: `>` collapsed, `<` expanded.
- Padding is measured past the accent bar, so the space either side of the
  label is visually equal.

### Interaction

- Pressing the tab expands it rightwards to reveal actions. **Identity never
  moves** when it expands.
- Press **arms** a control; release **activates** it, and only if the release
  lands on the same control. A press that drags off cancels.
- The pointer region covers the **tab only**.

### The parts model

Layout and hit-testing derive from **one measured list of parts**.

This is the central implementation decision, and it is a correctness decision
before it is a customization one. The prototype initially had three independent
copies of the same arithmetic - one to lay out, one to hit-test, one to size
the expansion - which have to agree about padding, tracking, icon pitch and
separator width forever. They disagreed three separate times during
prototyping. That is what the operator experienced as "clicking doesn't select
the item I click on" and "clicks sometimes don't register".

With one list, a part's hit box **is** the box it was drawn into, and there is
nothing to keep in sync.

Two properties fell out of writing the tests rather than the code, and both
were real defects:

- Inter-part padding was dead zone. Interactive parts now claim outward to the
  midpoint of the gap to their neighbour, and the first and last claim to the
  tab's edges, so no press inside the tab is swallowed.
- The pointer region is ceiled to whole pixels and so can be a pixel wider than
  the measured tab. Bounds-checking against the unrounded rect left a sliver
  that received events and resolved to nothing.

Customization then falls out: a part is a value in a vector, so reordering,
adding and removing parts is a config edit rather than an edit to the layout.

### Actions carry labels

Actions draw an **icon plus a text label** by default.

Three independent accessibility reviews rejected a row of bare glyphs. They
were right: a glyph can accelerate a label, but it cannot replace one, and the
actions that open further controls - audio level, USB device list, VM details -
cannot be guessed from a mark at all.

`compact-actions` restores the icon-only row for operators who have learned the
icons. It is recorded as a deliberate accessibility trade-off, not a supported
default.

The action vocabulary names outcomes rather than subsystems - `open-terminal`,
`audio-controls`, `usb-devices`, `vm-details`, `stop-vm` - because a config
that reads as a list of outcomes is one an operator can predict without
consulting a table. Each action declares whether it is **destructive** or
**opens further controls**, so a dispatcher cannot treat `stop-vm` as an
ordinary one-release activation.

### Two tiers of disclosure

One tier is not enough. A volume level, a USB device list and a VM's full
details cannot be expressed as icon toggles.

- **Tier 1** - the expanded row. Labelled, immediate actions.
- **Tier 2** - a proxy-drawn `xdg_popup` for actions whose `opens_submenu` flag
  is set, and for confirming destructive ones.

Tier 2 is drawn by the proxy rather than by a companion process. A companion
would introduce a second process that must be trusted to state identity
truthfully, and `zwlr_layer_shell_v1` cannot track a window anyway. Keeping
both tiers proxy-owned keeps the trust boundary exactly where it already is.

Tier-2 contents, all of which repeat the verified realm and workload:

| Opened by | Contents |
| --- | --- |
| `audio-controls` | Enable/disable, mute, output volume, input gain, device selection, and current state. |
| `usb-devices` | Devices with attached/available state, and attach/detach per device. Attachment names its target before commitment. |
| `vm-details` | Untruncated canonical identity, realm path, provider kind, state, address, closure drift, uptime, and capabilities. |
| `stop-vm` | A confirmation naming the realm and workload, defaulting focus to cancel. |

### Colour selection

Automatic label colour is chosen by **WCAG contrast ratio**, not brightness.

The proxy currently uses a weighted-sum brightness threshold. That omits the
sRGB transfer function and therefore over-estimates saturated colours. Measured
across 592 704 sampled colours it selects text below WCAG AA for **88 702** of
them, worst case **1.94:1** at `rgb(0, 216, 9)` - a colour an operator might
plausibly choose as a realm accent.

Choosing the better of black and white always clears AA, but only just: the
worst case is **4.58:1**, under 2% of margin. Both numbers are pinned by tests
in the proof so a regression in either direction is visible.

### Identity is realm and workload, from a trusted source

The label names **both the realm and the workload**, and comes from host-side
configuration - never from guest-supplied title text.

A realm-only label cannot distinguish two workloads in the same realm, and a
workload-only label cannot distinguish the same workload name across realms.
Colour cannot close either gap, because identity must survive a grayscale
render.

Truncation always preserves the realm. When the composed identity does not fit,
the workload portion ellipsizes and the realm does not, because losing the
realm is exactly the confusion this surface exists to prevent. The untruncated
canonical identifier is always available in `vm-details`.

### Identity text is a security surface

The label is what an operator reads before deciding which realm to type a
password into, so a workload name must not be able to lie about itself.

`char::is_control` is not sufficient. It covers Unicode category Cc, while the
overrides that reverse rendered text - U+202E and friends - are category Cf and
pass straight through. A workload named `work\u{202E}lamron` renders as
`worknormal`.

Sanitization therefore filters an explicit list chosen by the property that
matters ("changes what the reader sees") rather than by Unicode category, and
the proof asserts the gap so the list cannot be simplified back into a category
check. Ellipsization keeps the **start** of the name, because that is where
realm and workload identity live.

### Position is the defence against forgery

The proxy owns the band's pixels, so a guest cannot draw *in* the band. But a
guest controls every pixel below it, and a real toolkit can render a
pixel-exact copy of the tab. The adversarial capture
(`labs/window-chrome/SPOOF-FINDINGS.md`) stages both the obvious attack - a
fake tab flush beneath the real one - and the dangerous one: a fake window
frame, complete with its own tab and a password prompt, drawn inside a
different realm's window.

Appearance therefore cannot be the defence. **Position is.** The real tab is
always in the reserved band at the window's top-left, and the band is always
outside the guest's content rect. An operator who knows "the tab at the very
top of the window is the only real one" is not fooled by either attack.

Three things follow, and all three are binding:

1. The band boundary must be visually unambiguous, so "above the line" is a
   rule that can actually be applied.
2. The tab's **position is not configurable**. `PartsConfig` configures the
   tab's contents; identity is required to be the first part and the
   disclosure control to follow it immediately, both enforced at config load.
3. The rule is a documentation requirement as much as a rendering one.

### Bounded width

A labelled row can be wider than a narrow window. The row yields from the end:
identity and its disclosure control are never dropped, optional parts give way,
trailing separators are swept up, and an ellipsis marks what was hidden so the
operator can distinguish hidden from absent.

Every alternative is worse. Clipping cuts the identity label. Shrinking parts
breaks the target-size floor. Overhanging puts controls under the resize edge.
Refusing to decorate removes identity because an *optional* action did not fit.

### Failure is typed

There is no "draw nothing" arm. Layout returns either `Decorate` or
`FailClosed(reason)`.

This matters because the failure mode of an unlabelled proxied window is that
it becomes indistinguishable from an unproxied local window. A caller that
receives `FailClosed` must withhold the window or show a proxy-owned
placeholder; it must never fall through to showing guest pixels without a
label.

## Requirements for implementation

These come from the UX panel and are binding on the implementation waves. They
are recorded here because the prototype demonstrated the design, not the
finished product.

### Accessibility

| # | Requirement |
| --- | --- |
| A1 | Every interactive part meets 24×24 logical px (WCAG 2.2 SC 2.5.8) at every scale and in every configuration. Enforced during measurement so the hit box and the drawn box stay identical. |
| A2 | Label contrast ≥ 4.5:1 (SC 1.4.3); icon, control-boundary and state-indicator contrast ≥ 3:1 (SC 1.4.11). Enforced numerically for every configurable palette, with a defined fallback. |
| A3 | Identity survives a grayscale render: text carries identity, colour supports it (SC 1.4.1). |
| A4 | Font size, letter spacing and padding are scalable without breaking the measured layout (SC 1.4.12). A fixed 12 px label is not acceptable as the only option. |
| A5 | Actions carry text labels by default. Icon-only is opt-in and documented as a trade-off. |
| A6 | Keyboard access: a configurable host shortcut enters the chrome, with visible focus, traversal, Enter/Space activation, Escape collapse, and reliable focus return to the guest. This extends to tier 2: lists, sliders and confirm/cancel controls are all keyboard-operable, focus entry is deterministic, and focus is never trapped. |
| A7 | The AT-SPI gap is documented, not papered over. Screen readers cannot see proxy-drawn pixels; the existing `--title-prefix` identity path is retained as the accessible fallback for *identity*, and is explicitly **not** an equivalent for the *actions*. Before `--title-prefix` is described as an identity fallback in shipped docs, it is verified end to end under niri with a real AT-SPI screen reader. Every action requires either AT-SPI exposure or a verified screen-reader-operable equivalent. |
| A8 | Bidirectional overrides and other render-changing Unicode (U+202A-U+202E, U+2066-U+2069, zero-width and soft-hyphen characters) are removed from identity text before shaping. `char::is_control` covers category Cc only and does not catch these. |

### Security

| # | Requirement |
| --- | --- |
| S1 | Chrome pixels are proxy-owned. Guest buffers stay opaque and unsampled. |
| S2 | Identity-verification failure blocks guest pixels and renders a proxy-owned degraded placeholder. Visible guest content is never left unlabelled. |
| S3 | Realm accents are validated for mutual distinguishability, and identity remains legible without colour. |
| S4 | Destructive actions require a confirmation that names the realm and workload. |
| S5 | An adversarial capture - a guest drawing a pixel-matched fake tab directly below the real one, and a fake nested window - is produced and reviewed before the implementation wave closes. **Satisfied**: `labs/window-chrome/SPOOF-FINDINGS.md`. |
| S6 | Action dispatch belongs to the daemon. The surface that draws the icon reports the intent; it does not perform it. |
| S7 | The identity label names both realm and workload, sourced from host configuration and never from guest-supplied title text. Truncation always preserves the realm; the untruncated canonical identifier is available in `vm-details`. |
| S8 | Every tier-2 popup and every security-sensitive result repeats the verified realm and workload. A USB attachment names its target before commitment, and a terminal opened from the tab does not accept keyboard input until it is displaying verified chrome. |
| S9 | The tab's position is fixed and not configurable: identity is the first part and the disclosure control immediately follows it, enforced at config load. Position, not appearance, is what distinguishes the real tab from a guest forgery. |
| S10 | The band boundary is visually unambiguous, so "the tab above the boundary is the real one" is a rule an operator can apply. This rule is stated in operator-facing documentation. |

### Compositor behaviour

| # | Requirement |
| --- | --- |
| C1 | Size-hint translation for fixed- and minimum-size clients, so reserving the band can never silently crop guest content or violate declared geometry. Both minimum **and** maximum hints are translated by the measured band height, including fixed-size clients where min equals max. |
| C2 | Behaviour is captured and reviewed in niri's overview, tabbed and stacked column modes, dense scrolling rows, and short windows. |
| C3 | Unfocused windows keep their identity visible but collapse any expansion, so repeated action rows do not become visual noise. |
| C4 | Interaction with `clip-to-geometry`, window rules, and fractional scale is documented. |
| C5 | The tab is constrained to the available wrapper width. It never clips identity, never shrinks a target below its floor, never crosses a window edge, and never silently switches to compact mode. Optional parts yield from the end with a visible overflow mark. |

### Interaction

| # | Requirement |
| --- | --- |
| I1 | Distinct hover, armed, pressed and focus states. |
| I2 | Expansion collapses on Escape, on window deactivation, and when another window's tab is expanded. |
| I3 | Repeat-click handling: double-clicking disclosure does not expand-then-collapse, and a pending one-shot action cannot activate twice. |
| I4 | Tier-2 popup semantics are defined: placement relative to the invoking control, deterministic initial focus, traversal order, outside-click dismissal, Escape returning focus to the invoking control, dismissal on window deactivation, and a safe default focus (cancel) for destructive confirmation. |
| I5 | Dispatched actions have visible disabled, pending, success and failure states. A pending control stays locked until the daemon acknowledges, and a failure is surfaced next to the invoking control rather than only in logs. |
| I6 | Destructive actions are presented as destructive - last in the row, visually separated, and danger-styled - regardless of customization. `is_destructive` affects presentation as well as dispatch. |

### Customization

| # | Requirement |
| --- | --- |
| K1 | Typed Nix options under site defaults with per-VM overrides, with documented precedence and restart semantics. The generated JSON is a read-only implementation detail, not a hand-edited surface. |
| K2 | Invalid declarative configuration fails Nix evaluation with the full option path and remediation. Invalid *runtime* artifacts fall back to the labelled default row and surface a host-owned configuration-error indicator; journald alone is not visible enough. |
| K3 | A renderer-neutral style contract backed by `ui-colors.json` and mirrored into `ui-colors.css`: chrome surface, foreground, outline, focus, state tokens, and bounded global font/spacing/radius options. Light and dark palette behaviour is defined, and representative captures of both are reviewed. |
| K4 | Identity appears exactly once, is the first part, and the disclosure control immediately follows it; `expanded` must preserve `collapsed` as an ordered prefix. |
| K5 | Rows, spacers and total width are bounded, with actionable errors. |
| K6 | `activity-status` has a defined contract: its trusted data source, the composition and priority of its tokens, its empty state, its label, what activating it does, and whether it may be omitted. Safety-relevant activity - microphone in use, USB devices attached - is not suppressible by configuration. Capabilities the workload merely *has* belong in `vm-details`, not in the row. |
| K7 | The operator-facing Nix schema is documented alongside the wire form: canonical option paths, site and per-VM replacement semantics, `compactActions`, a typed spacer, bounds, and worked examples. `spacer:<px>` is generated wire data and is not the documented authoring form. |

## Configuration contract

Operators author in Nix. The generated JSON below is the wire form the
renderer reads; it is a read-only implementation detail, not a hand-edited
surface.

```nix
d2b.vms.work.graphics.waylandProxy.chrome = {
  collapsed = [ "identity" "disclosure" ];
  expanded  = [
    "identity" "disclosure" "separator"
    "open-terminal" "audio-controls" "usb-devices" "vm-details" "stop-vm"
  ];
  compactActions = false;
};
```

```json
{
  "collapsed": ["identity", "disclosure"],
  "expanded": [
    "identity", "disclosure", "separator",
    "open-terminal", "audio-controls", "usb-devices", "vm-details", "stop-vm"
  ],
  "compact-actions": false
}
```

Parts: `identity`, `disclosure`, `separator`, `activity-status`, `spacer`,
`spacer:<px>`, and the actions `open-terminal`, `audio-controls`,
`usb-devices`, `vm-details`, `stop-vm`. `overflow` is inserted by layout and is
not configurable.

Configurations are **validated, not merely parsed**. Refused, with a message
naming the valid parts:

- a row without `identity`, or with more than one - an unlabelled window is the
  failure this surface exists to prevent, and two identities are worse than
  none;
- a row that does not *begin* with `identity`, or where `disclosure` does not
  immediately follow it - position is what distinguishes the real tab from a
  forgery, so it is not negotiable;
- an `expanded` row that does not preserve `collapsed` as an ordered prefix,
  which would move identity at the moment the user is aiming at it;
- an `expanded` row that differs from `collapsed` with no `disclosure` in
  `collapsed` to open it;
- a duplicated interactive part;
- a row or spacer beyond its bound.

## Integration plan

| Target | Change |
| --- | --- |
| `packages/d2b-wayland-proxy` | Replace the rail path with the band + parts renderer. `WRAPPER_RAIL_WIDTH` and the rotated bitmap label path are removed. The wrapper's input region becomes the measured tab rect. `PointerFocus::Rail` becomes a tab focus that dispatches by part rather than swallowing. |
| `packages/d2b-core` | Adopt `PartsConfig`, `Part` and `Action` as bundle DTOs. The prototype's serde shapes were written to lift unchanged. |
| `nixos-modules` | Typed options for the parts rows and style tokens, per K1; emit the generated artifact through `nixos-modules/bundle-artifacts.nix` rather than hand-written install logic. |
| `packages/d2bd` | Action dispatch. The proxy reports intent; the daemon performs it, per S6. |
| `docs/reference/ui-colors.md` + `.json` | Add the chrome surface/foreground/outline/focus tokens, per K3. |
| `docs/reference/manifest-schema.*` | No change: chrome configuration is not per-VM manifest data. |
| `CHANGELOG.md` | `[Unreleased]` entry for the rail's removal and the option surface. |

The rail's removal is a **visible behaviour change** for existing consumers and
needs a migration note: windows gain a top band and lose the left rail, and
`d2b.vms.<vm>.graphics.waylandProxy.*` gains the chrome options.

## Alternatives considered

**Keep the rail, fix the font.** Rejected. Rotated text is only one of the four
defects; the input theft and the absence of anywhere to put actions are
structural.

**A tab outside window geometry, with niri's border wrapping it.** This was the
operator's original preference and it is the most visually appealing option.
Rejected because it is not implementable: client subsurfaces render above
niri's border, so the tab paints over the border rather than sitting inside it.

**A companion panel following the window.** Rejected: `zwlr_layer_shell_v1`
anchors to screen edges and cannot track a window.

**A GTK4 companion process rendering the menu.** Rejected. It introduces a
second process that must be trusted to state identity truthfully, and it cannot
render into the proxy's subsurface - one process cannot draw into another's.
The same reasoning rules out reusing `d2b-wlcontrol`'s Quickshell stack, which
is a separate-process layer-shell surface.

**An inset chip inside guest content, costing no geometry.** Rejected: it
overlaps guest content, and a guest that draws its own chip in the same place
makes identity ambiguous.

**Hover-to-reveal identity.** Rejected. Identity must be legible without
interaction; a realm marker that appears on hover is not a realm marker.

**Colour alone, no text.** Rejected on WCAG 1.4.1, on monochrome displays, and
arithmetically: an 8-colour palette with 0.15 grayscale separation is
impossible, since 7 × 0.15 exceeds 1.0.

## Consequences

- Every proxied window loses 32 logical px of height. This is the design's main
  cost and it is paid per window.
- Operators gain a place to put per-VM actions that is unambiguously bound to
  the window it acts on.
- Identity becomes legible without colour, at a real target size, with correct
  contrast.
- Edge-resize interaction is restored.
- The proxy gains a font-shaping dependency it did not have. Measured, the
  vector path costs ~3.4 MB resident against ~22.6 MB for a bitmap glyph cache
  on the same face, so this is a reduction rather than an addition.
- Screen readers still cannot see the chrome. Identity remains available
  through `--title-prefix`; the actions do not. This is a known, documented
  gap.

## Forward compatibility

Non-binding. d2b v3 (ADR 0046) models a `provider-wayland` artifact. Window
chrome configuration would map onto that provider's resource rather than onto
today's `d2b.vms.<vm>.graphics.waylandProxy.*`, and the parts DTOs are
transport-neutral, so the mapping is a relocation rather than a redesign. No v3
work is implied by this ADR.
