# d2b documentation

d2b documentation follows the [Diataxis] structure. Current product
documentation describes the Zone resource plane and controller-owned Guest
lifecycle. Historical migration and ADR pages remain available for context but
are not current configuration or command instructions.

## Tutorials and examples

- [`../templates/default/`](../templates/default/) - `nix flake init` scaffold.
- [`../examples/`](../examples/) - current Zone and Guest consumer flakes.
- [`../README.md`](../README.md) - product overview and quick start.

## Current references

- [`reference/zone-control-nix.md`](./reference/zone-control-nix.md) - Nix
  authoring for Zones, Guests, Providers, and immutable artifacts.
- [`reference/zone-cli-contract.md`](./reference/zone-cli-contract.md) - public
  daemon and CLI replacement contract.
- [`reference/cli-contract.md`](./reference/cli-contract.md) - current Rust
  CLI verbs, ResourceRefs, lifecycle, errors, and JSON behavior.
- [`reference/daemon-api.md`](./reference/daemon-api.md) - daemon protocol and
  controller lifecycle.
- [`reference/manifest-bundle.md`](./reference/manifest-bundle.md) - private
  bundle and artifact boundary.
- [`reference/manifest-schema.md`](./reference/manifest-schema.md) - versioned
  manifest compatibility contract.
- [`reference/error-codes.md`](./reference/error-codes.md) - typed refusal and
  remediation catalog.
- [`reference/store-lifecycle.md`](./reference/store-lifecycle.md) - Guest
  store views, restart adoption, locks, and cleanup.
- [`reference/key-lifecycle.md`](./reference/key-lifecycle.md) - managed
  identity and trust-state handling.
- [`reference/provider-capability-matrix.md`](./reference/provider-capability-matrix.md)
  - Provider capability and placement contracts.
- [`reference/display-io-capabilities.md`](./reference/display-io-capabilities.md)
  - mediated display, audio, clipboard, USB, and graphics boundaries.
- [`reference/components-graphics.md`](./reference/components-graphics.md),
  [`components-video.md`](./reference/components-video.md),
  [`components-audio.md`](./reference/components-audio.md),
  [`components-tpm.md`](./reference/components-tpm.md),
  [`components-usbip.md`](./reference/components-usbip.md), and
  [`components-shell.md`](./reference/components-shell.md) - Provider-specific
  effect contracts.
- [`reference/cli-output/`](./reference/cli-output/) - generated stable JSON
  schemas and prose companions.

## How-to

- [`how-to/install-nixos-tier1.md`](./how-to/install-nixos-tier1.md) - install
  d2b into a NixOS host.
- [`how-to/host-prepare.md`](./how-to/host-prepare.md) - prepare host
  prerequisites and ownership markers.
- [`how-to/create-provider.md`](./how-to/create-provider.md) - add a Provider
  crate and its declared artifacts.
- [`how-to/use-persistent-shells.md`](./how-to/use-persistent-shells.md) -
  open and manage Zone-scoped Guest shells.
- [`how-to/use-usb-security-key.md`](./how-to/use-usb-security-key.md) and
  [`how-to/troubleshoot-usbip.md`](./how-to/troubleshoot-usbip.md) - mediated
  device workflows.
- [`how-to/enable-observability.md`](./how-to/enable-observability.md) -
  Provider-backed telemetry.
- [`how-to/qemu-media.md`](./how-to/qemu-media.md) - QEMU media Provider
  integration.
- [`how-to/configure-desktop-terminal-integration.md`](./how-to/configure-desktop-terminal-integration.md)
  - desktop companion integration.

## Contributor and architecture guidance

- [`contributing/architecture.md`](./contributing/architecture.md) - current
  crate and control-plane ownership.
- [`contributing/critical-subsystems.md`](./contributing/critical-subsystems.md)
  - load-bearing invariants.
- [`contributing/gates-and-lints.md`](./contributing/gates-and-lints.md) -
  Bazel, Make, policy, fixture, drift, and Nix gates.
- [`../tests/AGENTS.md`](../tests/AGENTS.md) - test placement and retirement.

## Historical references

ADR 0015, ADR 0043, and the migration pages under `how-to/migrate-*` preserve
the rationale and upgrade history for retired lifecycle owners, but make no
v1/v2 state, data-retention, or rollback-preservation promise. Pages named
`realm-*`, `per-vm-*`, or `gateway-*` are historical unless explicitly linked
from the current references above. Do not copy their option paths or command
forms into a new configuration.

[Diataxis]: https://diataxis.fr/
