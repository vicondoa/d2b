# nixling examples

Five ready-to-eval consumer flakes plus two doc-friendly alias
directories. Read the per-directory README first.

| Path | Audience | Notes |
| --- | --- | --- |
| [`minimal/`](./minimal/) | Checked headless starter | Canonical flake behind the doc-friendly [`personal-dev/`](./personal-dev/) alias. |
| [`personal-dev/`](./personal-dev/) | Rust-first README alias | README-only pointer to `minimal/`; VM name `personal-dev`. |
| [`graphics-workstation/`](./graphics-workstation/) | Desktop VM with Wayland + audio + USBIP | Requires a Wayland compositor on the host. |
| [`multi-env/`](./multi-env/) (`demo`) | Two isolated envs (work + personal) | Demonstrates per-env isolation and route preflight (legacy v0.4.0 Tier 0 path). |
| [`multi-env/`](./multi-env/) (`multi-env-daemon-experimental`) | Two isolated envs + daemon opt-in | Exercises per-env `mtu` / `mssClamp` / `lan.allowEastWest` plus `supervisor = "nixlingd"` on one VM. |
| [`with-observability/`](./with-observability/) | Single workload VM + auto-declared observability stack | Grafana/Prometheus/Loki/Tempo on a dedicated `obs` env. |
| [`with-entra-id/`](./with-entra-id/) | Checked Entra-ID composition | Canonical flake behind the doc-friendly [`work-entra/`](./work-entra/) alias. |
| [`work-entra/`](./work-entra/) | Rust-first README alias | README-only pointer to `with-entra-id/`; VM name `work-entra`. |

## Alias-directory policy

`personal-dev/` and `work-entra/` intentionally do **not** ship a
`flake.nix`. They are lightweight alias READMEs so the docs can use
stable VM names while CI keeps one checked flake per scenario
(`minimal/` and `with-entra-id/`).

## `flake.lock` policy

Examples that are primarily meant to evaluate the in-tree framework
via `nixling.url = "path:../.."` do **not** commit a `flake.lock`
(currently `minimal/`, `graphics-workstation/`, `multi-env/`, and
`with-observability/`). Even when an example spells out shared inputs
such as `nixpkgs`, `microvm`, or `home-manager`, the point is still to
exercise the local checkout; a committed lock would be stale by
construction and `tests/static.sh` regenerates a local lock on first
eval anyway.

Examples that pull in an external sibling flake (`with-entra-id/`
consumes `github:vicondoa/entrablau.nix`) **do** commit their
`flake.lock` for reproducibility — the lock is the only way to ensure
the example builds bit-identically across machines.

## In-tree vs published consumption

Every checked example's `flake.nix` uses `nixling.url = "path:../.."`
so it can be evaluated against the in-tree framework without a
network round-trip. When you copy any of these layouts into your own
repo, swap that for a real flake ref — `github:vicondoa/nixling`
(track `main`) or a tagged release.

## See also

- [`../templates/default/`](../templates/default/) — `nix flake init`
  scaffold with sentinel TODOs + assertion gates.
- [`../README.md`](../README.md) — framework-level Rust-first quick
  start, threat model, and option index.
- [`../docs/`](../docs/) — reference docs (manifest schema, CLI
  contract, security runbook).
