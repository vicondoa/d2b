# UX-G1 round 3 - corrections

Round 2 returned **0/8**, but the findings collapsed onto a small, consistent
set. This round fixes them. Changes from round 2 are marked **[R2-fix]**.

## 1. Band height is a minimum with measured growth **[R2-fix]**

Raised by **8/8 seats** - the clearest signal the panel produced.

- **32 logical px is the minimum and the normal single-line height**, not a
  fixed height.
- Height is **measured from content**: `max(32, ceil(text_block + padding +
  accent_rule + focus_allowance))`. Two 14 px lines, or one line at 200% text
  scaling, grow the band deterministically. Identity is **never** clipped,
  compressed, or shrunk below the type floor to preserve a height.
- **Reflow order under pressure** (narrow windows, long labels, large text):
  1. Prefer a configured, uniqueness-checked **short display name**.
  2. Then wrap once at the realm delimiter.
  3. Then **move the status token into the menu**.
  4. Then **grow the band**.
  Identity always wins; the status token always yields first.
- **Honest cost, stated plainly:** every window pays its own band. A column of
  N stacked windows loses **N × band height** of guest area - 6 × 32 = 192 px
  at the minimum, more under text enlargement. This is the design's real price
  and the ADR states it in these terms.

## 2. One prioritized status token, not a badge strip **[R2-fix]**

Round 2 produced a genuine conflict: `design-taste` wanted badges gone
entirely; `security-ux` wanted *more* of them (mic-while-muted, `STOPPING`,
`DEGRADED`); `information-architecture` wanted exactly one slot;
`customization-ux` wanted a small typed registry.

Resolution - **a single slot showing the highest-priority
security-capability condition**, never an informational dashboard:

| Priority | Token | Shown when |
| --- | --- | --- |
| 1 | `UNVERIFIED` | Host-verified identity unavailable |
| 2 | `MIC` / `MIC MUTED` | Microphone **capability is granted or attached**, muted or not - the round-1 requirement, restored |
| 3 | `USB` | A USB device is attached |
| 4 | `STOPPING` / `DEGRADED` | Lifecycle or verification degradation |

Everything informational - pending restart, IP, uptime, closure drift - lives
in the menu, never on the band. The token is text + glyph, never colour alone,
never replaces the realm accent, and yields to identity under space pressure.

This satisfies all four positions: not a dashboard, capability exposure
preserved, exactly one slot, typed and closed.

## 3. Accessibility is a menu **shipping gate**, not a spike outcome **[R2-fix]**

Raised by 6/8. The spike stays, but its decision rule is now explicit and
fails closed:

- **P1 (proxy-painted popup) is diagnostic only.** It establishes the
  positioning and trust baseline and the AT-SPI floor (zero). **It may never
  ship as the interactive implementation**, regardless of how good it looks.
- **P2 ships only if it satisfies both** conditions simultaneously:
  (a) remains an `xdg_popup` parented to the trusted wrapper, tracking the
  window through niri's scrolling layout; and
  (b) passes verified AT-SPI acceptance - Name/Role/Value, `has-popup`,
  expanded state, actions, focus order, item semantics, and status
  announcements - tested with **Orca and Accerciser on niri**.
- **AT-SPI scope includes the identity button itself**, not only menu items.
- **If P2 proves infeasible**, the tab does **not** ship a painted menu.
  Activation instead opens the accessible `d2b-wlcontrol` surface, focused on
  the matching card. Tracking fidelity and AT semantics are not tradeable
  against each other.

## 4. Fail-closed means blocking, not labelling **[R2-fix]**

Round 2 established that the round-1 requirement was misread. `UNVERIFIED`
lettering above live, interactive guest content is *conspicuous failure*, not
fail-closed behaviour.

When host-verified identity is unavailable, the proxy **obscures guest content
and blocks guest input behind a host-owned interstitial** until verification
recovers. No realm colour is shown in this state.

## 5. Bidi mitigation is specified, not merely recorded **[R2-fix]**

Recording the confirmed vulnerability is not remediation. Before the title may
be treated as an identity channel, composition must:

- strip bidi **overrides and embeddings** (U+202A-U+202E, U+2066-U+2069) and
  other category **Cf** formatting characters, not just category Cc;
- **direction-isolate** the guest-controlled portion so it cannot reorder the
  host prefix;
- normalize and strip **reserved leading identity syntax** so a guest cannot
  emit `[work] …` from `personal`, and so prefixes cannot stack;
- use an unmistakable host-owned grammar (`d2b: work - <guest title>`);
- carry **RTL and visual-order acceptance cases**, plus an Orca announcement
  test confirming the host prefix is spoken first.

## 6. Labels: human name first **[R2-fix]**

Round 2 found the revision had misread this. Corrected:

- The band shows a **uniqueness-checked human display name** - `Work`, not
  `work.local.d2b`.
- **Provider kind is removed** from the default menu header; it does not answer
  "which environment owns this window?"
- Canonical targets, realm paths, and provider identifiers live in a technical
  details disclosure or in `d2b-wlcontrol`.
- Collisions among simultaneously visible identities are resolved with the
  shortest qualifying realm label, never by silent truncation.

## 7. Customization: contradictions removed **[R2-fix]**

- **`placement` is removed** from the theme schema. It contradicted the
  invariant that trusted-path position cannot vary. Raised by 4 seats.
- **Accent resolves exclusively from the existing `/etc/d2b/ui-colors`
  contract** (realm → environment → VM). It is renderer input, not a competing
  `windowChrome` theme value.
- **Per-VM exposes only** accent (via that contract) and a uniqueness-checked
  short display name. Typography, padding, and geometry are global-only.
- **A bounded typed status registry is restored** - `microphone`, `usb`,
  `pending-restart` - with fixed trusted data sources and closed action ids.
  No format strings, no intervals, no shell, no arbitrary click handlers. This
  honours the operator's "parts" intent within a safe envelope.
- GTK-only CSS is deferred entirely until a renderer is selected.

## 8. Slate and capture axes **[R2-fix]**

- **B is reclassified as a control**, not a candidate. It pays A's full 32 px
  cost while leaving a conspicuous dead strip. It is rendered to make that
  visible, not to be chosen.
- **A is the single candidate.** C, D, E, and now B are controls.
- **The outer decoration contract is fixed and tested**, not assumed: the proxy
  negotiates `ServerSide` so niri draws its ordinary surrounding border under
  `prefer-no-csd`. A and B are rendered under SSD, forced CSD, guest CSD, and
  both `draw-border-with-background` modes - because under CSD, niri's
  background-rect border may fill B's "transparent" band, making B not
  transparent at all.
- **niri layout states are captured, not asserted:** overview (compositor zoom
  can render a nominally persistent 12-14 px label unreadable), scrolling
  transition, floating, windowed-fullscreen, stacked columns, and tabbed
  columns alongside niri's native tab indicator.
- **Added axes:** narrow window · long label · 200% text scaling · status token
  present and absent · two- and three-window stacked columns.

## Unchanged from round 2 (no seat contested these)

Reserved top band inside declared window geometry · neutral surface with a 4 px
accent rule, never an accent fill · persistent horizontal identity in every
state including fullscreen · 14 px default / 12 px floor, weight 600 ·
input region 32×32 confined to chrome, never touching window edges or guest
content · drag-move on the band remainder, no drag-resize · menu reduced to
identity, conditional exceptional summaries, terminal, and `wlcontrol` handoff ·
no `custom/exec` · Nix as sole configuration authority · colour supportive,
text primary.

## Your task for round 3

Confirm your round-2 findings are resolved, or state precisely what is still
wrong. The design is now narrow; if it is sound, sign off. If you still have a
blocking finding, it should be a genuine defect rather than a preference -
several seats noted the remaining items were converging, and this round exists
to close them, not to accumulate new scope.

Return the same JSON shape. `signoff` is `true` iff `recommendations` is `[]`.
