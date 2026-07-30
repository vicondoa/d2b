# UX-G1 brief: window identity chrome for d2b

You are reviewing a **design slate**, before any of it is built. Your job is to
add, remove, merge, and sharpen the candidates so we build the right thing once.

## The product

d2b is a NixOS desktop framework that runs applications inside per-VM isolation
("work" VM, "personal" VM, a media VM, and so on). Their windows appear on the
host compositor - **niri**, a scrollable-tiling Wayland compositor - side by
side with each other and with host apps.

The user MUST be able to tell, at a glance, which VM a window belongs to.
Getting it wrong means typing a work credential into a personal-VM window.

A host-side Wayland proxy (`d2b-wayland-proxy`) sits between each guest app and
niri. It wraps every guest toplevel in a **proxy-owned wrapper `xdg_toplevel`**,
makes the guest surface a `wl_subsurface` of that wrapper, and paints
proxy-owned pixels around the guest content. The guest cannot see or forge
these pixels.

## What exists today, and why it fails

A 9-pixel-wide colored rail down the entire left edge of every window, with the
VM name drawn as a 5×7 bitmap font, rotated 90°, and stretched 1× horizontally
by 2× vertically to fit.

Screenshots (view these - they are the actual current product):

- `lab/out/baseline-rail-focused.png` - the rail on a focused window, freshly
  captured in a pinned nested niri (window 880×560, gaps 16, border 4px,
  corner radius 8, niri border `#7fc8ff`, VM accent `#ffa500`).
- `lab/out/baseline-rail-detail.png` - detail crop of the rail itself.

Confirmed defects, read from the source:

1. **Text is illegible.** Rotated 90°, non-uniformly scaled, 5×7 bitmap.
2. **Full window height.** The rail runs the whole left edge, colliding with
   niri's rounded corners and its gap/border language.
3. **It steals input.** The wrapper's input region is set to exactly the rail
   rect and the proxy *swallows* those events. That covers the entire left
   window edge, including where a user aims to drag-resize.
4. **It is a dead end.** Visual-only, so there is nowhere to hang actions.
5. **It occludes niri's own left border** (visible in the screenshot).
6. It fails WCAG 1.4.1 (color+unreadable text ≈ color alone), 1.4.3, 2.5.8
   (9 px target vs 24×24 minimum), 2.4.7 (no focus state).

## The goal

An **elegant, functional, minimal, accessible** identity affordance that:

- always says which VM/realm the window belongs to, in colour **and** readable
  text (monochrome monitors and low-vision users are explicit requirements);
- integrates with niri's visual language instead of fighting it;
- never blocks interaction it does not own;
- is **clickable**, opening a menu of per-VM actions;
- is customizable like waybar - ordered "parts", a declarative config, and a
  stylesheet sharing `/etc/d2b/ui-colors.css` with our sibling tools
  (`d2b-wlcontrol`, `d2b-clip-picker`, `d2b-wlterm`).

The operator's own words: *"something that they can associate with the window
… stays out of people's way but provides details and gives a place for
actions."* He also asked whether a tab could be positioned so niri's border
appears to wrap **around** it.

## Hard constraints from research (verified against niri source)

These are facts, not opinions. Design within them.

1. **Proxy subsurfaces render ABOVE niri's border.** In `tile.rs:render_inner`,
   window surface content is pushed before the border, so it is topmost. A tab
   drawn outside window geometry will paint *over* niri's border, never behind
   it. There is no way to put client pixels below the compositor border.
   → The "border wraps around the tab" effect is only achievable by the tab
   **painting its own border-coloured outline** to fake the detour.
2. **`clip-to-geometry` would erase an outside-geometry tab.** Default is off,
   but if the operator enables it, any subsurface outside the declared window
   geometry becomes invisible. **Mitigation: the proxy declares its own
   `set_window_geometry`, so it can declare geometry that INCLUDES the tab.**
   That makes the tab clip-immune, at the cost of niri allocating layout space
   for it and drawing its border around the tab-inclusive rect.
3. **niri's native tab indicator is the strongest precedent to match:**
   default 4 px wide, 5 px gap from the window edge, 50% of column length,
   Left position, drawn *outside* the window in the gap, colour falling back to
   the focus-ring/border colour. Configurable position (left/right/top/bottom),
   corner radius, gaps-between-tabs.
4. **niri honours `xdg_toplevel.move` and `xdg_toplevel.resize`** from clients,
   in both tiling and floating modes. A tab can therefore offer drag-move and
   drag-resize - niri has no native border-drag resize, so this is a genuine
   capability gain.
5. **Layer-shell surfaces cannot track a window.** `zwlr_layer_shell_v1`
   anchors to screen edges only; it cannot be positioned under a specific
   window or follow it as the tiling layout scrolls. **This is a serious blow
   to any "GTK4 companion panel anchored at the tab" design.** An `xdg_popup`
   on the proxy's wrapper toplevel *does* track the window correctly.
6. **niri's border is drawn outside the geometry rect**, symmetric, tile =
   window + 2×border. Corner radius shapes the border and (only when
   `clip-to-geometry` is on) the surface clip.
7. Guests are already denied `zwlr_layer_shell_v1`, so guests cannot forge
   chrome.

## Accessibility acceptance criteria (already pinned; treat as floor, not ceiling)

Colour **and** text always · text contrast ≥ 4.5:1 computed with the WCAG
relative-luminance formula (the naive `0.299R+0.587G+0.114B` currently in our
code is prohibited) · tab vs surroundings ≥ 3:1 · hit target ≥ 24×24 logical px
(32×32 preferred) · label ≥ 11 logical px, weight ≥ 500 · text-driven width, no
clipping at +0.12 em letter-spacing · visible keyboard focus state, ≥ 3:1 ·
identity survives a grayscale render · CVD-safe palette · no flashing, respect
`prefers-reduced-motion` · identity ALSO carried in the `xdg_toplevel` title
(`[work] Firefox`) and app_id, because screen readers cannot see proxy-painted
pixels - this is the only true AT channel.

## Prior art worth knowing

Qubes OS (coloured borders + VM name in the titlebar; the canonical secure-
labelling precedent) · Firefox Multi-Account Containers (coloured underline +
named chip) · Chrome profile pill · Windows Defender Application Guard (shield
+ coloured border) · niri's own tab indicator. Research notes: none of the
well-regarded systems use a non-interactive decorative rail that also steals
input; interactive identity indicators live in window chrome or an explicitly
interactive overlay. The prior-art analysis recommends a **top-left corner
chip** over a left-edge rail, because it does not consume the window's left
margin for the full height.

## The candidate slate (your primary review object)

Each variant renders colour + horizontal text, in focused / unfocused / urgent,
at scale 1 and 1.5, with a short (`work`) and long (`corp-workstation.work`)
label, on dark and light content, plus a grayscale pass.

| ID | Name | Placement | Geometry cost | The bet |
| --- | --- | --- | --- | --- |
| V1 | Notch tab | Top-left, outside window geometry, sitting on niri's top border | none | Reads as a folder tab |
| V2 | Inset chip | Inside content, top-left, 8 px inset, dims to a dot at rest | none | Zero layout impact |
| V3 | Header strip | Full-width strip inside geometry above content | ~22 px | Most titlebar-like |
| V4 | Corner wedge | Top-left corner inside geometry, label on hover | none | Smallest resting footprint |
| V5 | Edge stub | Short (~64 px) stub on the left edge, outside geometry | none | Left-edge identity without a full rail |
| V6 | Base capsule | Bottom-left capsule, outside geometry | none | Where the pointer rarely travels |
| V7 | Border plate | Geometry expanded minimally; tab colour/width/radius matched to niri's border so it reads as part of it | ~border | The literal "border goes around the tab" ask |
| V8 | Adaptive | Dot at rest → full label on hover/focus/keybinding | none | Progressive disclosure |

## The menu (opens on tab click)

Header (workload name, realm path, provider kind, state) · Audio: enable/disable
toggle, output volume, mic gain, mute · USB: attach → device picker, detach per
device · Info: IP, closure drift / pending restart, uptime · Actions: open
terminal, restart, stop (confirm) · full keyboard navigation, Esc dismisses.

## Customization model

waybar-shaped: `modules-left` / `modules-center` / `modules-right` of typed
parts (`identity`, `state`, `audio`, `usb`, `custom/exec`), each with `format`,
`tooltip`, `on-click`, `interval`; plus a stylesheet importing
`/etc/d2b/ui-colors.css` and reusing the `@d2b_state_*` / realm-accent names
that `d2b-wlcontrol` already ships.

## What we will build after this gate

Two menu implementations for comparison (proxy-drawn `xdg_popup` vs GTK4
companion), the winning tab variants, screenshots for operator selection, then
an ADR and a proof crate. Prototypes use fully synthetic data but real
interactions.

## Your task

Review the slate and everything above **as a design**, and return findings.
Specifically:

- Which variants should be **cut** outright, and why?
- Which should be **merged** or reframed?
- What is **missing** from the slate - is there an obviously better placement,
  form, or interaction model nobody listed?
- Where does the slate violate the constraints or the accessibility floor?
- Is the menu's content and structure right? What should be removed?
- Is the customization model right, or is it over-engineering?
- What single variant would you bet on, and what would make it fail?

Be specific and concrete. Vague praise is useless. If you think the whole
framing is wrong, say so and say what replaces it.

## Output format

Return **only** a JSON object:

```json
{
  "engineer": "<your role id>",
  "signoff": true|false,
  "summary": "What you reviewed and your overall posture.",
  "recommendations": [
    {
      "severity": "critical|high|medium|low",
      "title": "Short imperative title",
      "detail": "What is wrong and what to do instead. Be concrete.",
      "affects": ["V1", "V3", "menu", "customization", ...]
    }
  ]
}
```

`signoff` is `true` **iff** `recommendations` is `[]`. On a first-round design
review, findings are expected - do not sign off just to be agreeable, and do
not invent findings to look thorough.
