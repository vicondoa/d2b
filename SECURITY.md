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
- The Rust workspace (`packages/`) — bootstrap surface, supply-chain gates, and long-lived control-plane behavior.

Out of scope:
- Vulnerabilities in upstream `nixpkgs`, `microvm.nix`, `cloud-hypervisor`, `crosvm`, `swtpm` — report those to their respective maintainers; we'll coordinate.
- Vulnerabilities in consumer-side code that *uses* nixling (your own `/etc/nixos` is your concern; nixling provides primitives).
- Physical attacks (encrypted disk + TPM-bound unlock is a Lanzaboote concern, not nixling's).
- Side-channel attacks on shared CPU cache / SMT — out of scope (hardware-level concern).
- Supply-chain attacks on the Nix store (defer to upstream Nix + nixpkgs).

## Threat model

For the full threat model, see [`docs/explanation/design.md`](docs/explanation/design.md).

The short version: nixling defends against compromised-guest-userspace and cross-VM lateral movement. It does NOT defend against compromised host kernel, multi-user trust on a single host, or hardware-level adversaries.

### Portability roadmap

The portability work introduces a non-root `nixlingd` daemon plus a
minimal root-owned `nixling-priv-broker` (see ADRs 0001-0008 under
[`docs/adr/`](docs/adr/)). The new trust boundaries the daemon work
will introduce are:

- A single public CLI socket at `/run/nixling/public.sock`, mode
  `0660` group `nixling`. Membership in the `nixling` group (populated
  from `nixling.site.launcherUsers`) is the only *connection* gate —
  there is no second `nixling-admin` socket or group. Destructive /
  admin verbs (`vm exec`, `audit`, and `config sync`'s
  guest read) are gated a second time *inside the daemon*: the
  `SO_PEERCRED` peer identity must also appear in
  `nixling.site.adminUsers`. Authorization is `SO_PEERCRED` plus the
  system account database — never polkit at runtime.
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
  > per-share principal. Normal VM shares map to the per-VM runner
  > principal; the guest-control token share maps to the narrower
  > `nixling-<vm>-gctlfs` principal. The broker pre-establishes the
  > namespace via `clone3(CLONE_NEWUSER)` + `/proc/<pid>/uid_map`
  > writes before exec; virtiofsd runs fake-root only inside the
  > per-share user NS. This is strictly stronger than v1.1.1: a
  > compromised virtiofsd cannot access host resources outside its
  > bind-mounted share, even with kernel exploits that bypass the
  > sandbox, because the host kernel sees its credentials as the
  > unprivileged share principal — there are no in-host caps to
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

The v2 constellation layer ([ADR 0032](docs/adr/0032-nixling-v2-constellation-control-plane.md))
preserves the host's no-realm-egress posture: the **host** daemon and
broker still open no realm relay/provider connections. Realm egress
(relay rendezvous, provider APIs) is opt-in and confined to a per-realm
**gateway guest VM**; the host holds none of that realm's relay,
session, or provider credentials. A realm relay is treated as an
untrusted, ciphertext-only rendezvous: it sees connection metadata and
traffic shape but never plaintext operations, and relay credentials
authenticate relay access only — never a constellation principal or
local `Admin`. Gateway and provider-workload egress are expected to be
endpoint-allowlisted (relay/provider/registry endpoints only) and pinned
to an operator-approved tenant/subscription/region; this is realm
configuration held inside the gateway guest, not host state.

This section documents the daemon trust-boundary delta for consumers.

### Host-prepare trust-boundary delta

The broker's closed-enum surface covers host-prepare
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
  `nixling.site.audit.retentionDays` (set to `0` to disable
  pruning). **Reserved**: broker prune-on-rotate is shipping, but
  the NixOS option is not yet threaded into the broker invocation;
  broker uses the 14-day default regardless of overrides until
  then. The legacy `/var/lib/nixling/broker-audit.log`
  compatibility shim has been retired: both the writer
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
  `UsbipProxyReconcile`) is explicitly out of scope for this
  trust-boundary delta; only the per-busid
  `UsbipBindFirewallRule` skeleton is covered.

### Guest-control exec trust boundary

`nixling vm exec` runs a command inside a VM over
the authenticated guest-control vsock channel — there is no SSH. The
trust-boundary statements are:

- **Admin-only, destructive.** Guest exec is a destructive verb: the
  `SO_PEERCRED` caller must be in `nixling.site.adminUsers` (the
  daemon-side role gate above), on top of the `nixling`-group
  connection gate. Per-VM exec must also be enabled in the bundle
  (`guest.control.enable` + `guest.exec.enable`). Every exec runs the
  requested command as the VM's workload user (`ssh.user`) — **never
  root** — inside a real PAM login session (`systemd-run
  --property=PAMName=login --uid=<user>`); the wire `user` field is
  host-fixed by guestd and ignored, and operators elevate with `sudo`
  inside the session.
- **Leak-safe daemon-side audit.** The daemon records attached exec
  lifecycle events (`GuestControlExecEstablished` /
  `GuestControlExecTerminated`) to its own
  `daemon-events-<utc-date>.jsonl`, carrying ONLY the VM name, the
  admin `peer_uid`, and the negotiated `tty` shape. Detached create and
  kill/cancel write separate redacted daemon audit events carrying ONLY
  the VM name, admin `peer_uid`, closed action/result enums, and the
  opaque `exec_id`. The session handle, argv, env, cwd, exit status, and
  any stdin/stdout/stderr bytes are NEVER recorded. This daemon-side
  exec audit is distinct from the broker `OpAuditRecord` stream (which
  covers privileged host mutation, not guest exec).
- **Containment / DoS limits.** Exec is bounded at multiple layers:
  per-VM concurrent session caps; detached-exec slot and retained-log
  quotas; bounded per-op deadlines (each long-poll op gets a fresh
  deadline rather than an aging shared one); a hard in-flight op cap
  whose over-cap response is **close-only** — the owner session is torn
  down through the single existing teardown path with no reader-side
  socket write, preserving the single-writer invariant — so a stalled
  or abusive owner cannot pin unbounded work, and owner EOF/POLLHUP is
  always observed promptly; and bounded teardown on disconnect.
  Detached exec adds startup reconciliation, valid runner/workload
  re-adoption, orphan workload cleanup, terminal-record retention, and a
  periodic reaper that releases retained-log slots.

## See also

- [Design / threat model](docs/explanation/design.md)
- [`docs/explanation/design.md`](docs/explanation/design.md) — defense-in-depth list
- [CHANGELOG](CHANGELOG.md) — version history including security-relevant fixes
- [docs/reference/security-runbook.md](docs/reference/security-runbook.md) — operator incident-response, USBIP containment, and recovery procedures
