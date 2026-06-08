# Net VM bundle gate (`ph2-p2-net-vm-bundle-gate`)

The daemon refuses to start a `sys-<env>-net` VM (one of
`sys-corp-net`, `sys-personal-net`, `sys-obs-net`, …) when the
on-disk dnsmasq.conf hash for that env diverges from the hash the
trusted bundle implies. This catches the case where the bundle was
updated but the dnsmasq render step — a host singleton or systemd
unit — never reran, so the running net VM would silently serve a
stale lease table to its workloads.

## Trust boundary

| Property | Value |
| --- | --- |
| Runs in | `nixlingd` (unprivileged) |
| Invoked from | `dispatch_broker_vm_start`, BEFORE the host-prep DAG executes |
| Subject | `${dnsmasq_dir}/<env>.conf` (default `/var/lib/nixling/dnsmasq/<env>.conf`) |
| Capabilities used | — (pure `read()` against a file the daemon already has read access to) |
| Failure mode | refuses VM start with typed `daemon.bundle-dnsmasq-drift` envelope (exit code `63`) |
| Scope | net VMs (`is_net_vm = true` in `vms.json`); workload VMs short-circuit |

The check is a pure SHA-256 comparison; the daemon does not attempt
to repair the drift. Recovery is operator-driven (see
[Recovery](#recovery)).

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
| `ConfigMissing` | "dnsmasq.conf for env '<env>' is missing; bundle/dnsmasq render did not run" | Trigger the dnsmasq render step (host singleton or systemd unit), then retry. |
| `ConfigReadFailed` | "dnsmasq.conf for env '<env>' could not be read: <errno detail>" | Restore the file's ownership/mode (it should be daemon-readable). |
| `HashMismatch` | "dnsmasq.conf hash for env '<env>' diverges from bundle expectation (expected <sha256>, actual <sha256>); rebuild required" | The bundle was updated but the dnsmasq render step did not rerun. Re-render dnsmasq.conf and retry. |

All four are surfaced as the single typed-error variant
`TypedError::BundleDnsmasqDrift` with exit code `63` and kind
`bundle-dnsmasq-drift`. The full unredacted path is logged at
`warn!` level so operators can debug from `journalctl -u
nixlingd.service`; the public envelope intentionally omits it.

## Ordering

The preflight runs in `dispatch_broker_vm_start` AFTER bundle
resolver load and BEFORE:

* the host-prep DAG resolution (`build_host_prep_dag`) is logged;
* the per-VM ownership-matrix and ssh-host-key preflights run; and
* any broker mutating op fires.

This ordering is intentional: the failure surfaces *before* any
host mutation is attempted on behalf of the stale net VM. Workload
VMs (`is_net_vm = false`) short-circuit to `NotANetVm` with zero
filesystem reads.

## Recovery

The canonical recovery is `nixos-rebuild switch`, which re-runs the
dnsmasq render host singleton in addition to publishing a fresh
bundle. After the rebuild:

```bash
ls -l /var/lib/nixling/dnsmasq/<env>.conf
sha256sum /var/lib/nixling/dnsmasq/<env>.conf
nixling vm start sys-<env>-net --apply
```

For ad-hoc debugging without a full system rebuild, an operator
with admin role may invoke the render step directly (the step is
the host singleton currently named `nixling-net-render@<env>` —
exact name documented per host) and then retry the VM start.

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
