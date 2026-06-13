# Compatibility matrix

## Compatibility policy

`nixling` targets the exact `nixpkgs` revision pinned by the bundled
`flake.lock`. That lock is part of the supported surface.

If a downstream consumer makes `nixling` follow a different `nixpkgs`,
that combination is **unsupported**. The intended model is to make
companion flakes follow `nixling`'s `nixpkgs`, not to retarget
`nixling` to some other package set.

## Release matrix

The table below is derived from each release tag's `flake.lock` and the
release history in [`CHANGELOG.md`](../../CHANGELOG.md).

| nixling version | nixpkgs branch / channel | microvm.nix version | Host NixOS major version | Known incompatibilities |
|---|---|---|---|---|
| `0.3.0` | `nixos-unstable` (`d233902339c0`) | `77024c22f4dd` (locked git rev) | `nixos-unstable` | No release-specific incompatibility called out in the changelog beyond the global "do not mix nixpkgs" policy. |
| `0.2.0` | `nixos-unstable` (`d233902339c0`) | `77024c22f4dd` (locked git rev) | `nixos-unstable` | Manifest schema bumped to v2. Tooling built only for the v0.1.x manifest schema is incompatible. |
| `0.1.7` | `nixos-unstable` (`d233902339c0`) | `77024c22f4dd` (locked git rev) | `nixos-unstable` | No release-specific incompatibility called out; this is the first v0.1.x release where the sidecar restart policy works as documented. |
| `0.1.6` | `nixos-unstable` (`d233902339c0`) | `77024c22f4dd` (locked git rev) | `nixos-unstable` | GPU, swtpm, and audio sidecars still used the broken `unitConfig.X-RestartIfChanged` form. Upgrade to `0.1.7`. |
| `0.1.5` | `nixos-unstable` (`d233902339c0`) | `77024c22f4dd` (locked git rev) | `nixos-unstable` | Shipped the initial lifecycle-policy change, but three sidecars still needed the `0.1.7` restart-policy fix. Pre-`0.1.6` docs also use the legacy `[pending switch]` wording. |
| `0.1.4` | `nixos-unstable` (`d233902339c0`) | `77024c22f4dd` (locked git rev) | `nixos-unstable` | Predates the `nixling restart` / `pending-restart` workflow and later lifecycle fixes from `0.1.5`-`0.1.7`. |
| `0.1.3` | `nixos-unstable` (`d233902339c0`) | `77024c22f4dd` (locked git rev) | `nixos-unstable` | Predates the graphics/TPM bring-up fixes that landed in `0.1.4`. |
| `0.1.2` | `nixos-unstable` (`d233902339c0`) | `77024c22f4dd` (locked git rev) | `nixos-unstable` | Predates the `nixling@` wrapper and autostart fixes that landed in `0.1.3`. |
| `0.1.1` | `nixos-unstable` (`d233902339c0`) | `77024c22f4dd` (locked git rev) | `nixos-unstable` | Predates the `ConfigureWithoutCarrier` uplink-bridge fix from `0.1.2`; real host bring-up could deadlock. |
| `0.1.0` | `nixos-unstable` (`d233902339c0`) | `77024c22f4dd` (locked git rev) | `nixos-unstable` | First public alpha. Later `0.1.x` patch releases fixed consumer migration, networking bootstrap, wrapper/autostart, graphics/TPM, and lifecycle bugs. |

## Notes

- `microvm.nix` is not pinned by tag in `flake.lock`; this matrix
  reports the locked git revision instead.
- For numbered host-release expectations, read the `nixpkgs` input's
  branch name: e.g. `nixos-24.11` implies NixOS 24.11 hosts, while
  `nixos-unstable` is the rolling unstable branch.

## Host-prepare tier matrix (v0.4.0 baseline)

The privileged broker host-prepare contract defines tiers
gate which host verbs are supported per platform and what level of
pre-merge verification each row carries. The authoritative source
is [`docs/reference/support-matrix.md`](support-matrix.md); this
table is the at-a-glance summary.

| Tier | Platform | `nixling host check` | `nixling host prepare --dry-run` | `nixling host prepare --apply` | `nixling host destroy --apply` | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| **Tier 0** | NixOS x86_64 (NixOS-legacy host with no daemon-owned nixling bundle to reconcile) | supported | supported (reports `nothing-to-do`) | refused (`tier-0-legacy-uses-nixos-module`, exit 78) | refused | The per-VM `supervisor` option was removed in v1.1 (per ADR 0015); every enabled VM is daemon-supervised and uses the separate `nl-*`/`nlv-*` ifname space. |
| **Tier 1** | Ubuntu 24.04 LTS x86_64, kernel ≥ 6.6 | supported | supported | live in v1.0 (broker reconcile ops per ADR 0015) | live in v1.0 (broker reconcile ops per ADR 0015) | L3 pin: `tests/golden/l3-matrix/w3-ubuntu.txt`. NetworkManager 1.46, nftables 1.0.9, Cloud Hypervisor v40+, Nix-built minijail v17. |
| **Tier 1-later** | Fedora Server 40+ | supported (best-effort) | supported (best-effort) | live in v1.0 best-effort (broker reconcile ops per ADR 0015) | live in v1.0 best-effort (broker reconcile ops per ADR 0015) | L3 pin: `w3-fedora.txt`. v1.0 SLA only applies to Tier 0/1. |
| **Tier 2** | Arch Linux current, or other Linux x86_64 with cgroup v2 unified | supported (advisory) | supported (advisory) | live in v1.0 advisory (broker reconcile ops per ADR 0015) | live in v1.0 advisory (broker reconcile ops per ADR 0015) | Arch carries `w3-arch.txt`. Any unconfirmed prerequisite surfaces as `host-check-warning`. Operator reads the audit log + the per-distro troubleshooting anchor in `docs/how-to/host-prepare.md`. |

> **v1.0 status note (per [ADR 0015](../adr/0015-daemon-only-clean-break.md)).**
> v1.0 ships the `host prepare` / `host destroy` verbs with both
> `--dry-run` reconcile dispatch (read-only audit) and `--apply`
> wired live through the broker reconcile ops (`ApplyNftables`,
> `ApplyRoute`, `ApplySysctl`, `UpdateHostsFile`, `ApplyNmUnmanaged`).
> Broker failures surface as the typed `broker-error` envelope
> (exit 78, per
> [`docs/reference/error-codes.md`](./error-codes.md));
> daemon-unreachable surfaces `daemon-down` (exit 1). A Tier 0
> NixOS-legacy host (no daemon-owned nixling bundle to reconcile)
> returns the typed `tier-0-legacy-uses-nixos-module` envelope
> (exit 78). The historical staged-not-implemented disposition is
> retired now that the broker live ops have landed; see ADR 0015 and
> CHANGELOG.

The full ADR rationale for what is and is not supported lives in
[ADR 0008 — Supported platforms and rejected targets](../adr/0008-supported-platforms-and-rejected-targets.md).
