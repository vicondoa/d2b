# Security policy

## Supported versions

| Version | Status |
|---|---|
| v0.1.0 (alpha) | Supported — best-effort during alpha. |
| < v0.1.0 / pre-release | Not maintained. |

## Reporting a vulnerability

Please **do not** open public GitHub issues for security vulnerabilities.

### Channel: GitHub Security Advisory

File a private security advisory:
<https://github.com/vicondoa/nixling/security/advisories/new>

**For v0.1.0 (alpha), GitHub Security Advisories are the only
supported disclosure channel.** Email is not monitored and there is
no PGP key published. Future versions may add additional channels —
see the CHANGELOG for any expansion of the supported set.

GitHub's advisory tooling gates the disclosure timeline with
coordinated-disclosure primitives (private discussion, CVE
allocation, draft advisory) so a report filed there is the fastest
path to a coordinated fix.

## What to include

- A clear description of the vulnerability.
- Affected version(s) — commit hash or tag.
- Minimal reproduction (PoC if available, otherwise prose).
- Suggested severity (Critical / High / Medium / Low, optional).
- Disclosure preferences (timeline, attribution).

## What to expect

- Acknowledgment within 7 days (best-effort during alpha).
- An assessment + mitigation plan within 30 days.
- A coordinated-disclosure timeline negotiated case-by-case.
- A public advisory + CVE (where applicable) when the fix is ready.

## Scope

In scope:
- The nixling host-side modules (`nixos-modules/`).
- The nixling CLI (`nixos-modules/cli.nix`).
- The per-VM sidecars (`nixos-modules/host-sidecars.nix`, `nixos-modules/components/`).
- The framework's SSH key management (`nixling-keys` activation, virtiofs injection).
- Network isolation / NAT / firewalling (`nixos-modules/net.nix`, `nixos-modules/network.nix`).

Out of scope:
- Vulnerabilities in upstream `nixpkgs`, `microvm.nix`, `cloud-hypervisor`, `crosvm`, `swtpm` — report those to their respective maintainers; we'll coordinate.
- Vulnerabilities in consumer-side code that *uses* nixling (your own `/etc/nixos` is your concern; nixling provides primitives).
- Physical attacks (encrypted disk + TPM-bound unlock is a Lanzaboote concern, not nixling's).
- Side-channel attacks on shared CPU cache / SMT — out of scope (hardware-level concern).
- Supply-chain attacks on the Nix store (defer to upstream Nix + nixpkgs).

## Threat model

For the full threat model, see [`docs/explanation/design.md`](docs/explanation/design.md).

The short version: nixling defends against compromised-guest-userspace and cross-VM lateral movement. It does NOT defend against compromised host kernel, multi-user trust on a single host, or hardware-level adversaries.

## See also

- [Design / threat model](docs/explanation/design.md)
- [`docs/explanation/design.md`](docs/explanation/design.md) — defense-in-depth list
- [CHANGELOG](CHANGELOG.md) — version history including security-relevant fixes
