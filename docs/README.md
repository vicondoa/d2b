# nixling documentation

Organised as a [Diataxis] structure with four quadrants — tutorials,
how-to, reference, explanation. Today: **reference**, **how-to**, and
**explanation** docs ship in this tree, and **tutorials/examples**
live one level up under
[`../examples/`](../examples/) and [`../templates/default/`](../templates/default/).

## Tutorials / Examples

- [`../templates/default/`](../templates/default/) — `nix flake init`
  scaffold with sentinel TODOs and eval-time assertions. The
  fastest path to a working host.
- [`../examples/personal-dev/`](../examples/personal-dev/) —
  doc-friendly alias for the checked `examples/minimal/` flake;
  VM name `personal-dev`.
- [`../examples/graphics-workstation/`](../examples/graphics-workstation/) —
  desktop VM with Wayland + audio + USBIP YubiKey.
- [`../examples/multi-env/`](../examples/multi-env/) — two
  isolated envs side-by-side; demonstrates per-env isolation.
- [`../examples/work-entra/`](../examples/work-entra/) —
  doc-friendly alias for the checked `examples/with-entra-id/`
  flake; VM name `work-entra`.

[nei]: https://github.com/vicondoa/nixos-entra-id

The examples are intentionally small enough to read end-to-end;
each example's README explains the pattern.

## Reference

The contracts. Stable interfaces a consumer can depend on.

- [`reference/manifest-schema.md`](./reference/manifest-schema.md) —
  the per-VM JSON manifest the framework emits at
  `/run/current-system/sw/share/nixling/vms.json`. Field-by-field
  prose walkthrough + compatibility policy + example payloads.
- [`reference/manifest-schema.json`](./reference/manifest-schema.json) —
  the same contract as a JSON Schema (Draft 2020-12). The canonical
  type spec when the prose and the schema disagree.
- [`reference/compatibility.md`](./reference/compatibility.md) —
  release-by-release mapping of nixling tags to the bundled `nixpkgs`
  and `microvm.nix` pins, plus the support policy for downstreams.
- [`reference/cli-contract.md`](./reference/cli-contract.md) —
  the behavioural contract for any `nixling` CLI implementation
  (lifecycle FSM, signal semantics, exit codes, JSON vs human output).
- [`reference/error-codes.md`](./reference/error-codes.md) — the
  stable public error/refusal catalog for daemon, broker, and CLI
  surfaces.
- [`reference/store-lifecycle.md`](./reference/store-lifecycle.md) —
  per-VM hardlink-farm lifecycle, retention rules, crash-safety, and
  destructive-operation guardrails.
- [`reference/key-lifecycle.md`](./reference/key-lifecycle.md) —
  framework-managed SSH identity, trust-state, and audit behavior.
- [`reference/security-runbook.md`](./reference/security-runbook.md) —
  operator incident-response, USBIP emergency containment, and
  compromise-recovery steps.
- [`reference/error-envelope-guidance.md`](./reference/error-envelope-guidance.md) —
  daemon/broker/CLI envelope alignment, including broker-error
  remediation rules.
- **Per-component references** — one file per
  `nixos-modules/components/*.nix` toggle. Options, host-side
  resources created, runtime invariants, hardening notes, and the
  failure modes worth knowing about.
  - [`reference/components-graphics.md`](./reference/components-graphics.md) —
    `nixling.vms.<vm>.graphics.*` (virtio-gpu + Wayland cross-domain).
  - [`reference/components-video.md`](./reference/components-video.md) —
    optional graphics VM H264 decode via patched CH `--vhost-user-media`
    and patched crosvm `device video-decoder`.
  - [`reference/components-tpm.md`](./reference/components-tpm.md) —
    `nixling.vms.<vm>.tpm.*` (per-VM swtpm 2.0).
  - [`reference/components-usbip.md`](./reference/components-usbip.md) —
    `nixling.vms.<vm>.usbip.*` (YubiKey USBIP passthrough) plus the
    per-env host-side backend/proxy units.
  - [`reference/components-audio.md`](./reference/components-audio.md) —
    `nixling.vms.<vm>.audio.*` (vhost-user-sound + PipeWire) plus
    the `nixling audio` CLI surface.
  - [`reference/components-home-manager.md`](./reference/components-home-manager.md) —
    `nixling.vms.<vm>.homeManager.*` (Home Manager as a NixOS
    module inside the VM).

## How-to

Task-oriented recipes. Prescriptive, copy-and-adapt.

- [`how-to/install-nixos-tier1.md`](./how-to/install-nixos-tier1.md) —
  module-first Tier-1 install path for NixOS hosts.
- [`how-to/install-ubuntu-2404.md`](./how-to/install-ubuntu-2404.md) —
  current Ubuntu 24.04 manual/scaffold install path.
- [`how-to/install-fedora.md`](./how-to/install-fedora.md) —
  current Fedora manual/scaffold install path.
- [`how-to/host-prepare.md`](./how-to/host-prepare.md) —
  generic Linux Tier-1 onboarding and prerequisite reconciliation
  before daemon-managed VM lifecycle.
- [`how-to/migrate-nixos-to-daemon.md`](./how-to/migrate-nixos-to-daemon.md) —
  move a NixOS host from legacy systemd-owned VM lifecycle to
  `nixlingd`-owned lifecycle.
- [`how-to/migrate-nixling-v0-to-v1.md`](./how-to/migrate-nixling-v0-to-v1.md) —
  **primary v0.x → v1.0 operator runbook**. Manifest schema bump,
  bash CLI removal, per-VM systemd template retirement, host singleton
  retirement, polkit allowlist removal, default-switch auto-flip,
  whole-migration rollback. Also documents v1.1 deferred verbs and daemon-down
  rendering pointers (`audit` / `console` / `audio` / `keys`).
- [`how-to/uninstall-nixling.md`](./how-to/uninstall-nixling.md) —
  rollback and uninstall runbook for both NixOS and host-install
  scaffold paths.
- [`how-to/hardware-smoke-walkthrough.md`](./how-to/hardware-smoke-walkthrough.md) —
  operator procedure for the manual hardware/platform smokes.
- [`how-to/migrating-from-microvm.md`](./how-to/migrating-from-microvm.md) —
  port an existing `microvm.nix` deployment onto `nixling`: option
  mapping, step-by-step procedure, troubleshooting.
- [`how-to/write-a-nixling-addon.md`](./how-to/write-a-nixling-addon.md) —
  write a sibling flake that composes with `nixling` per VM, including
  the `nixpkgs` follow policy and eval-only test pattern.

## Explanation

Understanding-oriented prose. The "why" behind the design choices.

- [`explanation/design.md`](./explanation/design.md) — threat
  model, trust boundaries, component architecture, defense-in-
  depth controls, known gaps, and a *Why not X* rationale FAQ.
  Read this before opening a security-sensitive issue or
  proposing a structural refactor.

Operator runbooks that used to live under Explanation now live under
Reference so the day-2 procedures sit next to the stable CLI and
error contracts they rely on.

[Diataxis]: https://diataxis.fr/
