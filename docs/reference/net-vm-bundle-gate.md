# Net VM bundle gate (`ph2-p2-net-vm-bundle-gate`)

The daemon refuses to start a `sys-<env>-net` VM (one of
`sys-corp-net`, `sys-personal-net`, `sys-obs-net`, …) when the
on-disk dnsmasq.conf hash for that env diverges from the hash the
trusted bundle implies. This catches the case where the bundle was
updated but the dnsmasq render step — a host singleton or systemd
unit — never reran, so the running net VM would silently serve a
stale lease table to its workloads.

> **v1.1+ ordering note** (resolves R19 networking-r19-1). v1.1
> migrates the dnsmasq render step from the retired host-singleton
> path to a daemon-owned **`RenderDnsmasqEnvConf{env}`** broker
> op landed as part of the host-prep DAG in v1.1-P9b (per
> [ADR 0018](../adr/0018-microvm-nix-removal.md) and the v1.1
> plan P9b TDD-table row). The v1.0 `DnsmasqLeaseHashPreflight`
> currently runs at `dispatch_broker_vm_start` **BEFORE** the
> host-prep DAG; if the dnsmasq.conf has not yet been rendered
> (e.g., first boot after fresh install, or after a bundle
> update where the legacy render step was retired), the
> preflight fails-closed with `ConfigMissing` BEFORE the new
> `RenderDnsmasqEnvConf` host-prep op can run. v1.1-P9b
> implementation MUST reorder the preflight to run AFTER the
> `RenderDnsmasqEnvConf` op completes for the target env, OR
> change the preflight's `ConfigMissing` arm to soft-defer
> (mirroring the activation-script `daemon-down` soft-defer
> convention per ADR 0018) and re-run the preflight after the
> host-prep DAG renders the file. The plan v1.1-P9b TDD row
> for `tests/dnsmasq-env-conf-render-eval.sh` covers this
> ordering invariant explicitly: the test asserts that a fresh-
> install net-VM-start with no pre-existing
> `/var/lib/nixling/dnsmasq/<env>.conf` correctly invokes
> `RenderDnsmasqEnvConf` first, then re-runs the preflight,
> then proceeds with VM start (no `ConfigMissing` fail-closed).

## Trust boundary

| Property | Value |
| --- | --- |
| Runs in | `nixlingd` (unprivileged) |
| Invoked from | **v1.0**: `dispatch_broker_vm_start`, BEFORE the host-prep DAG executes. **v1.1+** (per the ordering note above): either reordered to AFTER `RenderDnsmasqEnvConf{env}` completes in the host-prep DAG, OR the `ConfigMissing` arm soft-defers and the preflight re-runs after host-prep — implementation choice is v1.1-P9b's per the plan TDD row. |
| Subject | `${dnsmasq_dir}/<env>.conf` (default `/var/lib/nixling/dnsmasq/<env>.conf`) |
| Capabilities used | — (pure `read()` against a file the daemon already has read access to) |
| Failure mode | refuses VM start with typed `daemon.bundle-dnsmasq-drift` envelope (exit code `63`); the `ConfigMissing` arm soft-defers in v1.1+ per the ordering note above |
| Scope | net VMs (`is_net_vm = true` in `vms.json`); workload VMs short-circuit |

The check is a pure SHA-256 comparison; the daemon does not attempt
to repair drift in the `HashMismatch` case (operator-driven recovery
per [Recovery](#recovery) below — re-render the dnsmasq.conf via
`nixling host prepare --apply` in v1.1+ or `nixos-rebuild switch` in
v1.0). In the `ConfigMissing` case, v1.1+ auto-repairs via the
`RenderDnsmasqEnvConf{env}` host-prep DAG op (per the v1.1+
ordering note above); v1.0 ConfigMissing remains operator-driven.

## Expected-hash computation

The expected hash is derived from three bundle-owned intent sources
exposed by `nixling_core::bundle_resolver::BundleResolver`:

1. `nft_intent[env:<env>]` — per-env nftables subset whose
   `desired_hash` already digests every bridge port-flag /
   forward-blocklist line that informs DHCP visibility.
2. `hosts_intent[host]` — the managed `/etc/hosts` block listing one
   line per env / bridge / MTU.
3. `route_intent[env:<env>:*]` — per-env route specs the net VM
   relies on for its uplink view.

The three sources are concatenated in a fixed, versioned canonical
form (prefix `nixling-dnsmasq:v1\n`) and hashed with SHA-256. The
encoding is sorted by intent id so the digest is byte-deterministic
across daemon restarts and across hosts running the same bundle.

```
nixling-dnsmasq:v1\n
nft:<nft env script body or "<absent>">\n
hosts:<hosts managed block or "<absent>">\n
routes:\n
  <route_spec for route:env:<env>:0>\n
  <route_spec for route:env:<env>:1>\n
  …
```

The full implementation lives in
[`packages/nixlingd/src/net_vm_bundle_gate.rs`](../../packages/nixlingd/src/net_vm_bundle_gate.rs).

## Actual-hash computation

The actual hash is `sha256(bytes_of_disk(${dnsmasq_dir}/<env>.conf))`,
also hex-encoded lowercase. The daemon does NOT parse or interpret
the file — only its bytes matter — so the rendering step is free to
choose any serialization that keeps producing the same bytes for the
same intent set.

The default parent dir `/var/lib/nixling/dnsmasq/` is overridable
via the `NIXLING_DNSMASQ_DIR` environment variable on the daemon
process, exclusively for hermetic tests. Production deployments
should leave the default.

## Refusal classes

| Drift variant | Refusal envelope `message` (after redaction) | Operator action |
| --- | --- | --- |
| `EnvMissing` | "net VM '<vm>' has no env in manifest" | Fix `vms.json`; rebuild and reactivate the bundle. |
| `ConfigMissing` | "dnsmasq.conf for env '<env>' is missing; bundle/dnsmasq render did not run" | **v1.0**: trigger the dnsmasq render step (host singleton or systemd unit), then retry. **v1.1+**: the daemon-owned `RenderDnsmasqEnvConf{env}` host-prep DAG op auto-renders the file on `nixling vm start --apply` per the v1.1-P9b implementation (`packages/nixling-host/src/dnsmasq.rs` + plan v1.1-P9b TDD row); the preflight either reorders after the render op completes OR soft-defers and re-runs on the next host-prep cycle. |
| `ConfigReadFailed` | "dnsmasq.conf for env '<env>' could not be read: <errno detail>" | Restore the file's ownership/mode (it should be daemon-readable). |
| `HashMismatch` | "dnsmasq.conf hash for env '<env>' diverges from bundle expectation (expected <sha256>, actual <sha256>); rebuild required" | The bundle was updated but the dnsmasq render step did not rerun. Re-render dnsmasq.conf and retry. |

All four are surfaced as the single typed-error variant
`TypedError::BundleDnsmasqDrift` with exit code `63` and kind
`bundle-dnsmasq-drift`. The full unredacted path is logged at
`warn!` level so operators can debug from `journalctl -u
nixlingd.service`; the public envelope intentionally omits it.

## Ordering

**v1.0 baseline:** the preflight runs in `dispatch_broker_vm_start`
AFTER bundle resolver load and BEFORE:

* the host-prep DAG resolution (`build_host_prep_dag`) is logged;
* the per-VM ownership-matrix and ssh-host-key preflights run; and
* any broker mutating op fires.

This v1.0 ordering is intentional: the failure surfaces *before*
any host mutation is attempted on behalf of the stale net VM.
Workload VMs (`is_net_vm = false`) short-circuit to `NotANetVm`
with zero filesystem reads.

**v1.1+ ordering update** (per the section preamble's v1.1
ordering note + plan v1.1-P9b TDD row): the v1.1
`RenderDnsmasqEnvConf{env}` daemon-host-prep DAG op now produces
the file rather than a retired host singleton. The preflight
ordering is one of two equivalent v1.1 implementations
(implementation choice belongs to v1.1-P9b): (a) reorder the
preflight to run AFTER `RenderDnsmasqEnvConf` for the target
env completes (preflight then sees a fresh file on every
start); OR (b) keep the v1.0 BEFORE-host-prep ordering but
change the `ConfigMissing` arm to **soft-defer** (similar to
the activation-script `daemon-down` soft-defer convention) so
the host-prep DAG runs, `RenderDnsmasqEnvConf` produces the
file, and the preflight re-runs and now succeeds. The
v1.1-P9b TDD gate asserts whichever path P9b lands.

## Recovery

**v1.0**: the canonical recovery is `nixos-rebuild switch`,
which re-runs the dnsmasq render host singleton in addition to
publishing a fresh bundle. After the rebuild:

```bash
ls -l /var/lib/nixling/dnsmasq/<env>.conf
sha256sum /var/lib/nixling/dnsmasq/<env>.conf
nixling vm start sys-<env>-net --apply
```

For ad-hoc debugging without a full system rebuild, an operator
with admin role may invoke the v1.0 render step directly (the
step is the host singleton currently named `nixling-net-render@<env>`
— exact name documented per host) and then retry the VM start.

**v1.1+**: recovery is automatic on `nixling vm start --apply`
because the v1.1-P9b `RenderDnsmasqEnvConf{env}` daemon-host-prep
DAG op runs as part of the start sequence. The retired
`nixos-rebuild`/`nixling-net-render@<env>` host singleton step
is no longer required (it is retired alongside the rest of
P6's host singletons per ADR 0015 + the v1.1-P12 doc-rot
sweep). If automatic recovery fails, the typed envelope
remains `daemon.bundle-dnsmasq-drift` (exit code `63`) and the
operator can manually invoke the host-prep DAG via
`nixling host prepare --apply` (which re-runs all daemon-owned
host-prep ops including `RenderDnsmasqEnvConf`) and then retry
the VM start.

## Tests

* Unit: `cargo test --lib net_vm_bundle_gate` exercises the eight
  cases enumerated in
  [`packages/nixlingd/src/net_vm_bundle_gate.rs`](../../packages/nixlingd/src/net_vm_bundle_gate.rs).
* Typed-error envelope: `cargo test --lib
  typed_error::tests::bundle_dnsmasq_drift_envelope_shape` pins
  exit code `63`, kind `bundle-dnsmasq-drift`, and remediation
  string.
* Integration:
  [`tests/net-vm-bundle-gate-eval.sh`](../../tests/net-vm-bundle-gate-eval.sh)
  wraps the cargo tests in the canonical static-gate shape used by
  the other P2 preflight gates.

## Cross-references

* [`docs/reference/privileges.md`](./privileges.md) §"Daemon-side
  P2 VM-start preflights" row `DnsmasqLeaseHashPreflight`.
* Plan `ph2-p2-net-vm-bundle-gate` — authoritative scope.
* [`packages/nixlingd/src/ssh_host_key_preflight.rs`](../../packages/nixlingd/src/ssh_host_key_preflight.rs) —
  sibling preflight (same trust boundary, different subject).
