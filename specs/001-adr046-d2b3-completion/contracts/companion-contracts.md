# Contract: Desktop companion surfaces

**Requirement**: FR-039, FR-040, SC-024 | **Waves**: W5 publish, W8 verify

## Why this file exists

FR-039 makes an unadapted companion a **release blocker**. A gate that names "every companion"
is unenforceable without a written, versioned set. No such inventory exists in the repository
today: two of the most contract-coupled companions are absent from the AGENTS.md sibling-flake
section entirely, and README names them only in passing prose.

Publishing this inventory is therefore itself a deliverable, not a lookup.

## The set

| Companion | Surface consumed | Adaptation risk |
| --- | --- | --- |
| `d2b-toolkit` | Client DTOs, public-socket framing, Wayland color parsing, Waybar helpers | **Highest** - shared substrate the others build on; must adapt first |
| `d2b-wlterm` | Public socket `ShellOp` family; capability discovery via `runtime.operationCapabilities.guest.shell`; launcher metadata; `d2b-wayland-proxy` package | High |
| `d2b-wlcontrol` | Public socket; `/etc/d2b/ui-colors.{json,css}`; `d2b audio status --json`; security-key state DTOs; graceful-stop semantics | High - deepest surface |
| `d2b-clip-picker` | Versioned newline-delimited JSON picker protocol over an inherited socketpair fd; realm target naming; accent colors | Medium |
| `weezterm` | None - supplies the terminal binary the launcher invokes | Low |

`entrablau.nix` is an identity sibling composed per-Guest, not a desktop companion.
`wl-proxy` is an upstream crate dependency, not a companion.

## Obligations

| # | Obligation | Wave |
| --- | --- | --- |
| CO-1 | Publish the companion inventory as a versioned reference document naming each companion, its exact consumed surface, and its verification status | W5 |
| CO-2 | Publish replacement contracts early enough for companions to adapt, given no preview release may be published (FR-045) | W5 |
| CO-3 | Verify each companion by exercising it against the release candidate on a live host - not by version number or source inspection | W8 |
| CO-4 | Hold the release while any companion lacks a verified compatible version | W8 |

## Tension to manage

FR-039 blocks the release on external repositories while FR-045 forbids publishing a preview
they could build against. The mitigation is CO-2: publish the *contracts* early even though no
*artifact* ships. If adaptation stalls anyway, the choice is between delaying the release and
amending FR-045 - and that amendment is a spec change, not an integrator judgment call.

## Acceptance

Every companion in the inventory has a compatible version exercised against the release
candidate on the daily-driver host before 3.0 is tagged (SC-024).
