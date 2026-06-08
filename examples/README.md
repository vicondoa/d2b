# nixling examples

Five ready-to-eval consumer flakes covering the headless,
graphics, multi-env, observability, and Entra-ID composition
cases. Each is self-contained — read the per-directory README first.

| Path                                          | Audience                                  | Notes                                                  |
|-----------------------------------------------|-------------------------------------------|--------------------------------------------------------|
| [`minimal/`](./minimal/)                      | Read-and-copy headless starter            | One env, one workload VM, ~25-line flake               |
| [`graphics-workstation/`](./graphics-workstation/) | Desktop VM with Wayland + audio + USBIP | Requires a Wayland compositor on the host             |
| [`multi-env/`](./multi-env/)                  | Two isolated envs (work + personal)       | Demonstrates per-env isolation and route preflight     |
| [`with-observability/`](./with-observability/) | Single workload VM + auto-declared observability stack | Grafana/Prometheus/Loki/Tempo on a dedicated `obs` env |
| [`with-entra-id/`](./with-entra-id/)          | Entra-ID-joined VM via the sibling flake  | Composes [`vicondoa/nixos-entra-id`][nei]              |

[nei]: https://github.com/vicondoa/nixos-entra-id

## `flake.lock` policy

Examples that are primarily meant to evaluate the in-tree framework
via `nixling.url = "path:../.."` do **not** commit a `flake.lock`
(currently `minimal/`, `graphics-workstation/`, `multi-env/`, and
`with-observability/`). Even when an example spells out shared inputs
such as `nixpkgs`, `microvm`, or `home-manager`, the point is still to
exercise the local checkout; a committed lock would be stale-by-
construction and `tests/static.sh` will regenerate a local lock on
first eval anyway.

Examples that pull in an external sibling flake (`with-entra-id/`
consumes `github:vicondoa/nixos-entra-id`) **do** commit their
`flake.lock` for reproducibility — the lock is the only way to ensure
the example builds bit-identically across machines.

## In-tree vs published consumption

Every example's `flake.nix` uses `nixling.url = "path:../.."` so it
can be evaluated against the in-tree framework without a network
round-trip. When you copy any of these layouts into your own repo,
swap that for a real flake ref — `github:vicondoa/nixling` (track
`main`) or `github:vicondoa/nixling/v0.1.0` once tagged releases
exist.

## See also

- [`../templates/default/`](../templates/default/) — `nix flake init`
  scaffold with sentinel TODOs + assertion gates. The fastest path
  to a working host.
- [`../README.md`](../README.md) — framework-level quick start,
  threat model, and option index.
- [`../docs/`](../docs/) — reference docs (manifest schema, CLI
  contract).
