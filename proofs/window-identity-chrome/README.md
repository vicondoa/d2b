# Proof: window identity chrome (ADR 0047)

The load-bearing logic behind ADR 0047, isolated from Wayland plumbing so it
can be tested directly.

```bash
cd proofs/window-identity-chrome && cargo test
```

72 tests. Standalone crate with its own lockfile, deliberately outside the
`packages/` workspace, matching the other proofs here.

## What it proves

| Module | Property |
| --- | --- |
| `geometry` | The band is reserved at the top, grows to hold its content rather than squashing it, and never shrinks below the 32 px target floor. The pointer region is the tab and nothing else — never the window edges, never guest content. Failure is typed: there is no "undecorated" success arm, so an unlabelled window cannot be the silent result of an edge case. |
| `parts` | Drawing and hit-testing share one measured list, so a part's hit box is the box it was drawn into. No part boxes overlap; every part is hit by its own centre; every x inside the tab resolves to a part. Identity appears exactly once and cannot move when the tab expands. |
| `contrast` | WCAG relative luminance, and the measured gap to the brightness test the proxy ships today. |
| `label` | Identity text cannot lie: bidi overrides and zero-width characters are removed, whitespace is collapsed, and ellipsization keeps the start of the name. |
| `action` | Actions declare whether they are destructive or open further controls, so a dispatcher cannot treat `stop-vm` as an ordinary activation. |
| `measure` | Text measurement abstracted from font rasterization, so layout properties hold for any shaper. |

## Two findings worth reading the tests for

**The shipped contrast test is not a contrast test.** `d2b-wayland-proxy` picks
label colour with a weighted-sum brightness threshold. That omits the sRGB
transfer function, so it over-estimates saturated colours. Measured over 592704
sampled colours, it selects text below WCAG AA for **88 702** of them, worst
case **1.94:1** at `rgb(0, 216, 9)` — a colour an operator might plausibly pick
as a realm accent. Choosing the better of black and white always clears AA, but
only just: the worst case is **4.58:1**, under 2% of margin, which is worth
knowing rather than assuming. Both numbers are asserted, so a regression in
either direction is visible.

**`char::is_control` does not catch the dangerous characters.** It covers
Unicode category Cc. The overrides that reverse rendered text — U+202E and
friends — are category Cf and pass straight through. A workload named
`work\u{202E}lamron` renders as `worknormal`. Since the label is what an
operator reads to decide which realm they are typing a password into, the
sanitizer filters an explicit list chosen by the property that matters
("changes what the reader sees") rather than by Unicode class.
`is_control_alone_would_not_have_caught_these` asserts the gap directly, so the
explicit list cannot be "simplified" back into a category check.

## What is not here

Pixel rendering, Wayland protocol handling, and anything needing a compositor.
Those live in the prototype under `labs/window-chrome/`, which runs against a
real niri session.
