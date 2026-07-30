# UX-G1 round 4 - closing findings, and a correction I owe the panel

Round 3: **4/8 sign-off** (`compositor-ux`, `information-architecture`,
`visual-design`, `customization-ux`). Four seats returned specific, closable
findings. This round closes them.

## 0. Correction: I gave the panel a false fact **[retracted]**

In rounds 2 and 3 I told you, labelled **CONFIRMED**, that:

> "for mid-tone fills, NEITHER black nor white text reaches 4.5:1. Small text on
> an arbitrary accent fill is unfixable by any auto-contrast rule."

**That is wrong.** I have since computed the bound properly, and the engine now
asserts the correct theorem:

- Contrast against black is `(L+0.05)/0.05`; against white it is
  `1.05/(L+0.05)`. They cross where `(L+0.05)² = 0.0525`, i.e.
  **L\* = 0.1791**, giving **4.5826:1**.
- A fine sweep of the colour cube confirms the analytic result: **choosing the
  better of black or white always clears 4.5:1, worst case 4.5826:1** (attained
  near `rgb(47,114,222)`).

So an accent fill with *correct* auto-contrast text **does** satisfy WCAG AA for
normal text. My claim that it was "unfixable" was false, and four seats may have
weighed it when signing off. I am flagging it rather than letting it stand.

**What is actually true, and still supports the neutral-plate decision:**

1. **The margin is razor-thin.** 4.5826 vs a 4.5 floor is **1.8% headroom** -
   no allowance for antialiasing, subpixel coverage, or fractional-scale
   blending, all of which move effective contrast at 12-14 px.
2. **The shipping proxy's naive luma rule produces real failures, not just
   disagreements.** Sweeping the cube, its choice falls below 4.5:1 for
   **262,386 sampled colours**, worst case **1.94:1** at `rgb(4,216,0)` - a
   bright green that sits just under the naive threshold and therefore gets
   white text. That is a severe, shipping WCAG violation.
3. `visual-design`'s round-1 argument stands on its own: per-colour text
   flipping across a palette is fragile and visually incoherent regardless of
   whether each individual pairing technically passes.

The neutral-plate-plus-accent-rule decision therefore stands, but on **margin
and coherence** grounds, not on the impossibility I incorrectly asserted.

**Seats that signed off in round 3: please confirm your sign-off still holds
given this correction.** If it changes your view, say so.

## 1. Status slot: one fixed contract, no registry **[R3-fix, design-taste]**

You caught a genuine contradiction between §2 (fixed priority list, no
pending-restart) and §7 (configurable registry including pending-restart with
action ids). §7's registry is **removed**. The slot is now a single fixed,
non-configurable contract:

- Not configurable. No registry, no action ids, no ordering knobs.
- **Pending restart is menu-only**, permanently.
- Token activation always opens the **same identity menu** - the token is never
  a separate control with separate behaviour.

## 2. Concurrent capabilities compose; they do not hide **[R3-fix, security]**

One slot, but the slot's *content* composes rather than showing only the
top-priority condition:

- **Verification state takes its own precedence** and is never composed away:
  `UNVERIFIED` and `DEGRADED` render alone.
- Otherwise capabilities compose in a bounded token:
  `MIC · USB`, `MIC MUTED · USB`.
- **Security-capability state never yields into the menu under space
  pressure.** It grows or reflows the band instead. The reflow order is
  corrected accordingly:

  1. configured short display name → 2. wrap once at the realm delimiter →
  3. **grow the band** → (informational content was already menu-only, so
  nothing security-relevant is ever dropped).

  The earlier "status yields first" rule applied to a badge *strip* that no
  longer exists.

## 3. No clickable AT-invisible control, ever **[R3-fix, security + accessibility]**

Round 3 correctly found that falling back to `wlcontrol` does not repair the
identity **button's** missing Name/Role/State/Action. Corrected:

If P2 cannot deliver AT semantics for the identity button itself, then the
chrome is **non-interactive** - it renders identity only, and `wlcontrol` is
reached through an accessible compositor command. A clickable control with no
AT semantics is not shipped in any configuration. The AT-SPI shipping gate
covers **the identity button, the status token, the popup, its items, and the
interstitial** - not menu items alone.

## 4. The blocking interstitial is accessible **[R3-fix, accessibility]**

The host-owned blocking state gets a defined AT channel: an Orca-announced
host-owned title plus AT-SPI status/alert semantics stating that content is
blocked and why. A screen-reader user must never meet an apparently focused
guest that silently rejects input. Included in the Orca + Accerciser gate.

## 5. Horizontal overflow has a guaranteed terminal strategy **[R3-fix, accessibility]**

Band growth cannot rescue a long unbroken label in a narrow window at 200% text.
The terminal rule:

1. A **uniqueness-checked compact display name** with a **measured maximum
   width** is required for every identity.
2. Safe additional wrapping is permitted, with corresponding height growth.
3. A **wrapper minimum width** is enforced. If the compact name cannot fit even
   that minimum, the proxy **refuses to decorate** rather than clipping,
   overlapping, shrinking, or overflowing identity. (The prototype engine
   already implements refusal: `resolve()` returns `None`.)

Identity never clips, never overlaps, never shrinks below the type floor, never
overflows.

## 6. Labels are stable for the session **[R3-fix, security]**

Collision resolution runs once over **all configured identities**, not over the
currently visible set. The resolved label is fixed for the session. A tab is
never renamed because another window appeared or a workspace changed.

## 7. Button interaction contract **[R3-fix, interaction]**

- Pointer cursor over the button; distinct **hover**, **pressed**,
  **menu-open**, and **keyboard-invoked** treatments, none of which reduce
  identity contrast.
- **Activate on release inside** the button. Cancel on drag beyond threshold or
  release outside.
- **Right-click** opens the same menu. **Middle-click and wheel do nothing** -
  no hidden volume gestures.
- These states join candidate A's capture and acceptance axes.

## 8. Popup behaviour contract **[R3-fix, interaction]**

P2 must, and P1 measures the same as its positioning baseline:

- anchor **below the identity button**, aligned to its start edge;
- **flip or slide** at output edges;
- **follow the window while niri scrolls**, without pointer jumps;
- **toggle** on repeat activation;
- dismiss on outside click, `Escape`, parent destruction, or invalidation;
- **restore focus to the guest window** on dismissal;
- bounded scrollable height, stable width.

## Unchanged

Everything not listed above is as stated in round 3 and drew no round-3
objection: the reserved top band inside declared geometry, measured height with
a 32 px floor, neutral surface with a 4 px accent rule, persistent horizontal
identity in every state including fullscreen, 14 px default / 12 px floor at
weight 600, a 32×32 input region confined to chrome, drag-move without
drag-resize, the reduced menu, bidi-safe title composition, human display names,
Nix as sole configuration authority, candidate A with controls B-E, and the full
decoration and niri layout capture matrices.

## Your task for round 4

Two questions:

1. **Seats that signed off in round 3** - does the §0 correction change your
   position? Re-affirm or withdraw.
2. **Seats with open findings** - are they closed?

Return the same JSON shape. `signoff` is `true` iff `recommendations` is `[]`.
