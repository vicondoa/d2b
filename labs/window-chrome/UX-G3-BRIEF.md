# UX-G3 — finalist review

You are reviewing the **finalist** design for d2b's window identity chrome
(ADR 0047), as an *interactive* surface. This gate is the last UX gate before
the ADR is written.

## What you must do

1. **Look at the images.** Use the `view` tool on each file listed below. A
   reviewer who has not seen the artifact cannot judge it.
2. Review the design, the interaction model, and the customization model.
3. Return the JSON sign-off record described at the end.

**Do not run tests, builds, or long validations.** The integrator has already
run them and the results are given below. If validation is missing or
insufficient, say so as a finding rather than running it yourself.

## Images (view all of these)

Directory: `/home/paydro/projects/d2b-window-chrome/labs/window-chrome/out/g3-20260729-025314/`

| File | State |
| --- | --- |
| `collapsed.png` | At rest. Default parts row. |
| `expanded-default.png` | Expanded. Default row: terminal, audio, usb, info, stop. |
| `expanded-custom.png` | Expanded, with an operator config selecting only audio, usb, terminal — in that order. |
| `long-label.png` | Long workload name (`corp-workstation.work`). |
| `personal-realm.png` | A second realm, to check identity legibility across realms. |

These are captured from the operator's **real niri session**, not a mock: real
border colours (active `#dbb7ff`), real scale, real window gaps, real content
behind.

## What was replaced

`d2b-wayland-proxy` previously drew a 9 px full-window-height "rail" on the
left edge of every proxied guest window:

- the label was a 5×7 bitmap font, rotated 90°, and stretched non-uniformly
  (x1, y2) to fit 9 px;
- it ran the full window height, colliding with niri's rounded corners;
- its input region covered the entire left window edge and **swallowed** the
  events, so edge interaction was blocked;
- it was visual-only, so there was nowhere to hang per-VM actions.

A 9 px hit target also fails WCAG 2.2 SC 2.5.8.

## The finalist

A **reserved band at the top of the window, inside declared geometry**,
containing a single left-aligned tab.

- The band is 32 px minimum, grown from measured row heights when content
  needs more.
- The tab is three concentric rounded shapes: an outer border-coloured shape,
  a card inset uniformly from it, and a 3 px accent bar formed from the card's
  own left columns clipped to the card. Concentric construction is why the
  outer and inner curves agree.
- Fill is a neutral off-gray (`#25272b`) matching `d2b-wlcontrol`'s chip
  colour; the realm accent appears as the left bar plus a hairline elsewhere
  (accent mixed 45% toward the fill).
- The label is horizontal, 12 px, optically centred on cap height.
- A chevron follows the label: `>` collapsed, `<` expanded.
- Padding is measured **past** the accent bar so the visible space either side
  of `Work >` is equal.
- Pressing the tab expands it rightwards to reveal action icons. Identity never
  moves.

### Interaction

- Press **arms** a control; release **activates** it, and only if the release
  lands on the same control. A press that drags off cancels.
- The pointer region covers the **tab only** — never the window edges, never
  guest content. Edge resize and drag are restored.
- Layout and hit-testing are derived from **one measured list of parts**, so
  the box a control was drawn into is the box that receives its presses.
- No press inside the tab is swallowed: interactive parts claim outward to the
  midpoint of the gap to their neighbour, and the first and last claim to the
  tab's edges.

### Customization

Waybar-shaped, but as a **generated** artifact (d2b's convention is that Nix is
the sole authority and emits JSON that serde reads):

```json
{
  "collapsed": ["identity", "chevron"],
  "expanded": ["identity", "chevron", "separator", "audio", "usb", "terminal"]
}
```

Available parts: `identity`, `chevron`, `separator`, `status`, `spacer`,
`spacer:<px>`, and the actions `terminal`, `audio`, `usb`, `info`, `stop`.

Configs are **validated, not merely parsed**. Refused, with a message naming
the valid parts:

- a row without `identity` (an unlabelled window is the security failure this
  surface exists to prevent);
- an `expanded` row that differs from `collapsed` with no `chevron` in
  `collapsed` to open it;
- a duplicated part.

An invalid config falls back to the **default row**, logged as
`chrome-parts-rejected`, rather than rendering nothing.

## Constraints that are fixed (do not relitigate)

These came out of source-cited niri research and were confirmed empirically:

- **Client subsurfaces render above niri's border.** A tab drawn outside window
  geometry paints *over* the border. "niri's border wraps around a protruding
  tab" is therefore impossible. This killed the notch/edge-stub/base-capsule
  placements.
- **niri does not advertise `zxdg_decoration_manager_v1`** (absent from all 41
  globals, verified by logging). The `draw-border-with-background false` window
  rule is the only mechanism, not a workaround.
- **`zwlr_layer_shell_v1` anchors to screen edges only** and cannot track a
  window. This killed any anchored companion panel.
- Because the tab lives *inside* declared geometry, `clip-to-geometry true`
  **improves** it (rounds its corners) rather than erasing it.
- The guest cannot draw the tab: chrome pixels are proxy-owned, and guests are
  denied `zwlr_layer_shell_v1`.

## Validation already run

- `d2b-chrome-engine`: **117 tests pass**. Includes: no part box overlaps; every
  part is hit by its own centre; every x inside the tab resolves to a part (no
  dead zones); leading and trailing padding are equal; 2× metrics give 2× width;
  output is premultiplied BGRA; identity-unverified refuses to render;
  reordering parts moves their hit boxes with them.
- Vendored prototype proxy: **156 tests pass**. Includes an end-to-end check
  that the drawn tab's actions resolve in order across the whole pointer region
  with no dead zones, and that the region never reaches the window edges.
- Live on the operator's desktop: the custom config above renders exactly its
  three icons in its configured order.

### Known gaps (call these out if they matter to your seat)

- Screen readers cannot see proxy-drawn pixels. The existing `--title-prefix`
  path is the accessible fallback and is retained; the AT-SPI gap is real.
- The band costs 32 px of vertical window real estate per window.
- An adversarial spoof capture (a guest attempting to imitate the tab) has not
  yet been produced.
- Keyboard access to the tab is not yet designed.
- Overview, tabbed and stacked niri layouts have not been captured.

## Your focus

{FOCUS}

## Return format

Return **only** this JSON:

```json
{
  "engineer": "{SEAT}",
  "signoff": true,
  "summary": "What you reviewed and the overall posture.",
  "recommendations": []
}
```

`signoff` is `true` **iff** `recommendations` is `[]`. Each recommendation must
be concrete and actionable, and carry a severity: prefix it with `CRITICAL:`,
`HIGH:`, `MEDIUM:` or `LOW:`. Do not raise style nits or restate the brief.
