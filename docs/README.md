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
- [`../examples/minimal/`](../examples/minimal/) — read-and-copy
  headless starter: one env, one workload VM.
- [`../examples/graphics-workstation/`](../examples/graphics-workstation/) —
  desktop VM with Wayland + audio + USBIP YubiKey.
- [`../examples/multi-env/`](../examples/multi-env/) — two
  isolated envs side-by-side; demonstrates per-env isolation.
- [`../examples/with-entra-id/`](../examples/with-entra-id/) —
  composition with the sibling [`nixos-entra-id`][nei] flake
  for Entra-joined VMs.

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
- [`reference/cli-contract.md`](./reference/cli-contract.md) —
  the behavioural contract for any `nixling` CLI implementation
  (lifecycle FSM, signal semantics, exit codes, JSON vs human output).
- **Per-component references** — one file per
  `nixos-modules/components/*.nix` toggle. Options, host-side
  resources created, runtime invariants, hardening notes, and the
  failure modes worth knowing about.
  - [`reference/components-graphics.md`](./reference/components-graphics.md) —
    `nixling.vms.<vm>.graphics.*` (virtio-gpu + Wayland cross-domain).
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

- [`how-to/migrating-from-microvm.md`](./how-to/migrating-from-microvm.md) —
  port an existing `microvm.nix` deployment onto `nixling`: option
  mapping, step-by-step procedure, troubleshooting.

More recipes ("rotate a VM's SSH key", "add a second env", …) land
alongside the same directory in subsequent phases.

## Explanation

Understanding-oriented prose. The "why" behind the design choices.

- [`explanation/design.md`](./explanation/design.md) — threat
  model, trust boundaries, component architecture, defense-in-
  depth controls, known gaps, and a *Why not X* rationale FAQ.
  Read this before opening a security-sensitive issue or
  proposing a structural refactor.

[Diataxis]: https://diataxis.fr/
