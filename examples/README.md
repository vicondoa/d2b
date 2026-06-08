# nixling examples

Four ready-to-eval consumer flakes covering the headless,
graphics, multi-env, and Entra-ID composition cases. Each is
self-contained — read the per-directory README first.

| Path                                          | Audience                                  | Notes                                                  |
|-----------------------------------------------|-------------------------------------------|--------------------------------------------------------|
| [`minimal/`](./minimal/)                      | Read-and-copy headless starter            | One env, one workload VM, ~25-line flake               |
| [`graphics-workstation/`](./graphics-workstation/) | Desktop VM with Wayland + audio + USBIP | Requires a Wayland compositor on the host             |
| [`multi-env/`](./multi-env/)                  | Two isolated envs (work + personal)       | Demonstrates per-env isolation and route preflight     |
| [`with-entra-id/`](./with-entra-id/)          | Entra-ID-joined VM via the sibling flake  | Composes [`vicondoa/nixos-entra-id`][nei]              |

[nei]: https://github.com/vicondoa/nixos-entra-id

## `flake.lock` policy

Examples whose only flake input is `nixling.url = "path:../.."`
(i.e. `minimal/`, `graphics-workstation/`, `multi-env/`) do **not**
commit a `flake.lock`. There is nothing external to pin — the only
input resolves to the in-tree framework via a relative path. A
committed lock for such an example would be stale-by-construction
and add noise to every `flake.lock` review.

Examples that pull in external inputs (`with-entra-id/` consumes
`github:vicondoa/nixos-entra-id`) **do** commit their `flake.lock`
for reproducibility — the lock is the only way to ensure the
example builds bit-identically across machines.

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
