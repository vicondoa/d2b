# Adversarial capture: can a guest forge the identity tab?

Produced by `bin/capture-spoof.sh`. Satisfies requirement **S5** in
[ADR 0047](../../../docs/adr/0047-window-identity-chrome.md).

## The question

The proxy owns the band's pixels by construction - the guest cannot draw
there. That is not the interesting question. The interesting question is
whether a guest that controls every pixel *below* the band can produce
something an operator would act on.

Both attacks are staged with plain terminal escape sequences, which is the
weakest possible attacker. A real toolkit could render a pixel-exact copy of
the tab. That the terminal's forgery looks slightly cruder is therefore **not**
the defence, and must not be mistaken for one.

## `spoof-adjacent.png`

A `Work` window whose guest draws a `Personal` tab flush beneath the real one.

Two tabs are visible. The real one sits in the band above the guest's content
boundary; the forgery sits below it.

## `spoof-nested.png`

The more dangerous attack: the guest draws an entire fake window frame,
complete with its own `Personal` tab and a password prompt, inside a `Work`
window. An operator who reads the nearest tab rather than the topmost one
types a personal credential into a work VM.

## The finding

**Position is the defence, not appearance.**

The real tab is always in the reserved band at the window's top-left corner,
and the band is always outside the guest's content rect. Everything below the
band boundary is guest content by construction. An operator who knows "the
tab at the very top of the window is the only real one" is not fooled by
either attack; an operator who reads the nearest tab is fooled by both.

This has three consequences for the implementation:

1. **The band boundary must be visually unambiguous.** The guest content must
   be seen to start somewhere definite, so "above the line" is a rule an
   operator can actually apply.
2. **The tab's position must be fixed, not configurable.** Allowing an
   operator to move the tab would destroy the invariant that makes the rule
   teachable. `PartsConfig` deliberately configures the tab's *contents*, not
   its placement.
3. **This is a documentation requirement as much as a rendering one.** The
   rule has to be stated somewhere the operator will read it.

## What this does not show

- A pixel-exact forgery by a real toolkit. The terminal cannot round corners
  or draw sub-cell geometry; a GTK or Qt guest can. Assume the forgery is
  perfect and the finding above still holds, which is why it is stated in
  terms of position.
- Multi-window confusion, where a guest opens a second real window to
  reinforce the illusion.
- Behaviour under niri's overview, where windows are scaled down and the band
  boundary is proportionally smaller.
