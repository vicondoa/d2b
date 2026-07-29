# UX-G1 round 2 — revised design

Round 1 returned **0/8**. This is the revision. Every cut below is traceable to
a finding; nothing was dropped for convenience.

## What round 1 killed

- **V2, V4, V8** — identity that disappears at rest. 8/8.
- **V5** — left-edge contention; collides with niri's native tab indicator
  (default Left, x = −9 px). 7/8.
- **V6** — weak scan priority and association. 7/8 (one seat allowed it as an
  optional placement; not carried).
- **The waybar module model** — `modules-left/center/right`, `format`,
  `interval`, `tooltip`, arbitrary `on-click`. 8/8.
- **`custom/exec`** — the proxy must never execute user shell strings or render
  their output in trusted identity chrome. 8/8.
- **The anchored GTK companion tab** — `zwlr_layer_shell_v1` anchors to screen
  edges and cannot track a window through niri's scrolling layout. 7/8.
- **Menu as a control centre** — sliders, USB picker, IP, uptime, closure
  detail, restart, stop. Moved to `d2b-wlcontrol`.

## The geometric fact that dissolved the V1/V7-vs-V3 split

Window geometry is **rectangular**. Reserving space for a top-left plate expands
the bounding rectangle across the entire window width and costs the plate's full
height regardless. "Top-left plate with reserved geometry" and "reserved band
with a left-aligned chip" are therefore the *same allocation*; the only genuine
choice left is whether the band is painted or transparent.

It also settles the operator's question: **niri's border cannot literally wrap
around a protruding tab.** Client subsurfaces render above the compositor
border, so a tab can only paint over it. Reserving geometry makes niri draw its
ordinary border around the tab-inclusive rectangle — the honest version of the
wish, and the better-looking one.

## The design

**Name:** the *window identity tab*.

**Geometry.** Reserve **32 logical px** at the top of the wrapper's declared
window geometry; guest content is offset below. 32 rather than 22–28 because the
input region must be 32×32 and must live entirely inside reserved chrome,
overlapping neither guest content nor resize zones.

**Composition, left to right:**

- **Identity button** — visible face ≥ 24 px high, input region 32×32.
  Neutral surface, high-contrast neutral text, **4 px VM accent rule**. The
  accent is never the text background.
- **Label** — canonical lowercase (`work`, `corp-workstation.work`), 14 px
  default / 12 px absolute floor, weight 600, text-driven width, survives
  +0.12 em tracking and 200% text scaling. The distinguishing portion is never
  auto-ellipsized; long labels wrap once at the realm delimiter, or use a
  configured, uniqueness-checked short name. The full name always remains in the
  menu, the window title, and the accessible name.
- **State badges, right-aligned** — text + glyph, present only when the
  condition is real: `MIC` while capture is actually active, `USB` while a
  device is attached, `UPDATE` for pending restart. Sourced from trusted daemon
  state, never from the guest.
- **Remainder of the band** — optional drag-move region with a drag threshold.
  No drag-resize.

**Always visible.** Focused, unfocused, fullscreen, windowed-fullscreen, and in
the overview. Emphasis may vary; presence and contrast may not.

**Fail closed.** When host-verified identity is unavailable, show a host-owned
`UNVERIFIED` state with a distinct pattern and **no realm colour**. Never
present guest content as a known realm.

**Colour is supportive, not primary.** Round 1 proved the original palette
criteria impossible: eight identity colours cannot have pairwise grayscale
luminance gaps of 0.15, because seven intervals need a range of 1.05 and the
available range is 1.0. Persistent unique **text** is the identity; colour and a
tested glyph/shape reinforce it. This is independently confirmed by the engine's
contrast tests: for mid-tone fills, *neither* black nor white text reaches
4.5:1, so small text on an arbitrary accent fill is unfixable by any
auto-contrast rule — hence neutral plate plus accent rule.

## Prototype slate (2 candidates, 3 controls)

Deliberately small. The controls exist to make the argument **visible** to the
operator rather than asserted in prose.

| ID | Kind | What it is | Question it answers |
| --- | --- | --- | --- |
| **A** | candidate | 32 px band **painted** neutral chrome; identity button left; accent rule; badges right | Does a painted band read as trusted chrome or as a second titlebar? |
| **B** | candidate | Same reserved geometry, band **transparent** except the identity button; niri borders the full rect | Does the empty band look deliberate or broken? |
| **C** | control | Identity button with **accent fill** + auto-contrast text | Makes the contrast failure visible instead of theoretical |
| **D** | control | **Outside-geometry** notch, no reservation | Shows the sticker effect, the border overpaint, and erasure under `clip-to-geometry true` |
| **E** | control | The **current rail**, already captured | Before/after reference |

**Axes rendered for A and B:** focused · unfocused · unverified · fullscreen ·
short label (`work`) · long label (`corp-workstation.work`) · dark and light
guest content · scale 1 and 1.5 · grayscale pass · +0.12 em tracking pass ·
badges present and absent.

## The menu, reframed as an accessibility spike

Round 1 produced a genuine conflict rather than a consensus:

- Positioning says **proxy-owned `xdg_popup`**: it tracks the window by
  construction, the guest cannot cover it, and layer-shell cannot do this.
- Accessibility says a raw proxy-painted popup has **no AT-SPI at all** — no
  role, name, expanded state, or menu-item semantics — and that keyboard
  navigation without AT-SPI is not accessibility.

Both seats independently proposed the same resolution: an **accessible
toolkit-backed surface that is still an `xdg_popup` of the trusted wrapper**.
Whether that is achievable is an open technical question, so the prototype
treats it as a spike and measures both paths:

1. **P1 — proxy-painted `xdg_popup`.** Tracking, trust, and dismissal
   behaviour. Establishes the positioning baseline and the AT-SPI floor (zero).
2. **P2 — accessible toolkit surface.** Real AT-SPI role/name/state, verified
   with Orca and Accerciser on niri. Measures what it costs in tracking fidelity.

Deliverable is evidence for the ADR, not two shipped implementations.

**Menu content (both paths):** identity header (full name, realm, provider,
textual state) · conditional exceptional summaries (`Microphone active`,
`1 USB device attached`, `Update pending`) · `Open terminal in <name>` ·
`Open <name> controls…` handing off to `d2b-wlcontrol` at the right card. No
sliders, no device picker, no IP/uptime, no restart/stop.

**Keyboard.** Not inserted into the guest's Tab order. A compositor keybinding
opens the focused window's menu; arrows/Tab navigate, Enter activates, Esc
dismisses, focus returns to the guest, no focus trap.

## Customization, reduced

Nix is the **sole** authority: `d2b.site.ui.windowChrome` for defaults,
`d2b.vms.<vm>.graphics.waylandProxy.chrome` for per-VM overrides. Any runtime
file is serialization, not a second configuration surface.

A typed, **renderer-neutral theme-token schema** replaces free CSS so both
rendering paths can honour it: surface, foreground, accent, outline, focus
colour, radius, padding, gap, font family/size/weight, placement. Optional
GTK-only CSS is explicitly non-portable and may not restyle the protected
identity node.

**Invariant regardless of configuration:** identity presence, position, text,
minimum size, contrast floors, focus treatment, and activation. Styling that
would hide the label, shrink the target, or fail contrast is rejected at
evaluation time.

Reuse `/etc/d2b/ui-colors.css` tokens verbatim. Note the confirmed live drift
before copying it: d2b emits `d2b_state_pending_restart`, but
`d2b-wlcontrol/data/style.css:31` references the non-existent
`@d2b_state_pendingRestart`. New neutral chrome tokens
(`d2b_chrome_surface`, `_foreground`, `_outline`, `_focus`) must be added to
both the JSON and CSS contracts.

## Verified code findings carried into the ADR

1. **Bidi overrides survive title sanitization.**
   `sanitize_rewritten_label` (`packages/d2b-wayland-proxy/src/policy.rs`)
   strips ANSI escapes and `char::is_control()`, but `is_control()` covers only
   Unicode category Cc. U+202E RLO, U+202D LRO, and U+2067 RLI are category Cf
   and pass through — verified empirically. The title is a load-bearing identity
   channel, so this is a spoofing vector.
2. **Title prefixes stack.** A guest in `personal` that titles itself
   `[work] Firefox` yields `[personal] [work] Firefox`. Fix: a host-owned
   grammar plus stripping of reserved leading identity syntax.
3. **A dead CSS token ships today** (the `pendingRestart` drift above).

## Acceptance-criteria corrections adopted

- **Removed:** the ΔL ≥ 0.15 grayscale rule (arithmetically impossible for 8
  identities) and the "kill/shutdown action" requirement (destructive process
  termination is not an accessibility floor).
- **Raised:** type floor from 11 px to 14 px default / 12 px absolute; visible
  face ≥ 24 px with a 32×32 input region.
- **Downgraded:** `app_id` from an accessibility requirement to grouping
  metadata.
- **Reworded:** accessible name to contain the visible label
  (`work VM actions`, not `click for options`) for Label-in-Name.
- **Added:** keyboard operability and no-trap, logical focus order, pointer
  cancellation, dragging alternatives, Name/Role/Value, status announcements,
  focus restoration, 200% text scaling, high-contrast operation, and an
  **Orca + Accerciser acceptance test on niri** rather than asserting the title
  channel works.

## Your task for round 2

Review this revision against your round-1 findings.

- Were your findings addressed, ignored, or misread? Say which.
- Does the converged design introduce **new** problems your round-1 review did
  not anticipate?
- Is the 2-candidate / 3-control slate right, or still wrong?
- Is the menu accessibility spike the right way to resolve the popup conflict?
- Is 32 px the right reservation? What is the honest cost in a stacked column?

Return the same JSON shape. `signoff` is `true` iff `recommendations` is `[]`.
Sign off if the design is now sound — do not manufacture findings to appear
rigorous, and do not withhold sign-off over preferences you would not block on.
