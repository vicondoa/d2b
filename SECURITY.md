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
- The W0a/W0b Rust workspace (`packages/`) — bootstrap surface and supply-chain gates only; long-lived control-plane behavior lands in W2+ with its own SECURITY scope update.

Out of scope:
- Vulnerabilities in upstream `nixpkgs`, `microvm.nix`, `cloud-hypervisor`, `crosvm`, `swtpm` — report those to their respective maintainers; we'll coordinate.
- Vulnerabilities in consumer-side code that *uses* nixling (your own `/etc/nixos` is your concern; nixling provides primitives).
- Physical attacks (encrypted disk + TPM-bound unlock is a Lanzaboote concern, not nixling's).
- Side-channel attacks on shared CPU cache / SMT — out of scope (hardware-level concern).
- Supply-chain attacks on the Nix store (defer to upstream Nix + nixpkgs).

## Threat model

For the full threat model, see [`docs/explanation/design.md`](docs/explanation/design.md).

The short version: nixling defends against compromised-guest-userspace and cross-VM lateral movement. It does NOT defend against compromised host kernel, multi-user trust on a single host, or hardware-level adversaries.

### Portability roadmap (W0b scope draft)

The portability work introduces a non-root `nixlingd` daemon plus a
minimal root-owned `nixling-priv-broker` (see ADRs 0001-0008 under
[`docs/adr/`](docs/adr/)). The new trust boundaries the daemon work
will introduce are:

- A public CLI socket at `/run/nixling/nixlingd.sock` ACL'd to
  `nixling-launcher` (daily lifecycle) and `nixling-admin`
  (destructive ops), authenticated by `SO_PEERCRED` plus the system
  account database — not by polkit at runtime.
- A private broker socket at `/run/nixling/priv.sock` reachable only
  by the `nixlingd` service uid. The broker re-derives every
  privileged parameter from its own copy of the root-owned bundle
  and writes an append-only root-owned audit log.
- Per-role minijail profiles for every VM runner and sidecar, with
  declared uid/gid, capability sets, namespace plan, bind mounts,
  seccomp policy, and cgroup placement. `requiresStartRoot` is
  permitted only for audited carve-outs.

  > **v1.1.2 update** ([ADR 0021](docs/adr/0021-broker-user-namespace-for-virtiofsd.md)):
  > the virtiofsd `requiresStartRoot=true` carve-out from
  > [ADR 0003](docs/adr/0003-minijail-provisioning-and-sandbox-interface.md)
  > is RETIRED. virtiofsd profiles now declare zero host
  > capabilities (`capabilities = []`), `requiresStartRoot = false`,
  > and a `userNamespace` block mapping in-NS UID/GID 0 to the
  > per-VM runner principal. The broker pre-establishes the
  > namespace via `clone3(CLONE_NEWUSER)` + `/proc/<pid>/uid_map`
  > writes before exec; virtiofsd runs fake-root only inside the
  > per-runner user NS. This is strictly stronger than v1.1.1: a
  > compromised virtiofsd cannot access host resources outside its
  > bind-mounted share, even with kernel exploits that bypass the
  > sandbox, because the host kernel sees its credentials as the
  > unprivileged runner principal — there are no in-host caps to
  > escalate from.

The first non-NixOS target is Ubuntu 24.04 LTS x86_64 with kernel
>= 6.6, Nix daemon install, KVM, cgroup v2 unified hierarchy,
nftables, NetworkManager, Cloud Hypervisor, and a Nix-built minijail.
macOS/vfkit, WSL, containers as hosts, Alpine/musl, non-systemd
autostart, rootless Nix, Firecracker feature parity,
crosvm-as-full-VMM parity, and aarch64 runtime graphics/audio are
**explicitly rejected** at the first milestone; adding any of these
requires a new ADR + panel sign-off.

Telemetry posture is preserved: `nixlingd` makes no outbound network
connections by default; any future opt-in lands behind an explicit
`--enable-diagnostics` flag and an update to this file.

W2 ships the full rewrite of this section once the daemon code is in
place; W0b only ships this scope draft so consumers can read the
trust-boundary delta before the implementation lands.

### W3 trust-boundary delta (host-prepare wave)

W3 extends the broker's closed-enum surface to cover host-prepare
mutation: cgroup v2 delegation + pidfd handoff (ADR 0011), per-link
sysctls + bridge/TAP + NetworkManager unmanaged config + `/etc/hosts`
managed-block + route preflight (ADR 0012), `inet nixling` nftables
table apply + USBIP firewall-rule skeleton (ADR 0013), and
`modprobe`/device-node opens + runner-shape preflight (ADR 0014).
The full operation catalog with audit/destructive/secret flags is
[`docs/reference/privileges.md`](docs/reference/privileges.md); the
conceptual model + recovery runbook is
[`docs/explanation/host-prepare.md`](docs/explanation/host-prepare.md).

The new trust-boundary statements are:

- The broker mutates network, cgroup, sysctl, `/etc/hosts`,
  NetworkManager unmanaged config, and `modprobe` state on behalf
  of `nixlingd`, gated entirely by the closed broker enum plus the
  trusted bundle. Every operation has a typed handler under
  `packages/nixling-priv-broker/src/ops/` and re-derives its
  operating paths from the bundle, never from caller input.
- Compromise of `nixlingd` cannot escalate to arbitrary host
  mutation beyond the declared broker enum variants. Unknown
  variants and unknown fields in security-sensitive artifacts are
  refused (`defaultForUnknown: deny`).
- The broker audit log
  (`/var/lib/nixling/audit/broker-<utc-date>.jsonl`) is
  root-owned, append-only via a pre-opened `O_APPEND` fd, and
  rotated daily. Retention defaults to 14 days, overridable via
  `nixling.site.audit.retentionDays` (W4a-H1; set to `0` to
  disable pruning). **Reserved at W4a-H1**: broker prune-on-
  rotate is shipping, but the NixOS option is not yet threaded
  into the broker invocation (W4 main wave); broker uses the
  14-day default regardless of overrides until then. The pre-W3
  legacy `/var/lib/nixling/broker-audit.log` compatibility shim
  has been retired (W4 retire-shim): both the writer
  (`AuditLog::write_entry` and `AuditLog::write_op_record`) and
  the reader (`AuditLog::export_lines`, which now enumerates the
  full daily-file directory in chronological order) operate
  solely against `broker-<utc-date>.jsonl` files — see
  [`docs/reference/daemon-api.md`](docs/reference/daemon-api.md#audit)
  "Retention" and "Legacy retirement".
- An admin can pause the broker (`nixling admin broker --pause`);
  the post-compromise rotation/repair runbook lives at
  [`docs/explanation/host-prepare.md`](docs/explanation/host-prepare.md)
  § Recovery runbook.
- USBIP live device routing (`UsbipBind`/`UsbipUnbind`/
  `UsbipProxyReconcile`) is explicitly out of W3 scope; W3 ships
  only the per-busid `UsbipBindFirewallRule` skeleton.

## See also

- [Design / threat model](docs/explanation/design.md)
- [`docs/explanation/design.md`](docs/explanation/design.md) — defense-in-depth list
- [CHANGELOG](CHANGELOG.md) — version history including security-relevant fixes
- [docs/reference/security-runbook.md](docs/reference/security-runbook.md) — operator incident-response, USBIP containment, and recovery procedures
