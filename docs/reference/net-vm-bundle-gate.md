# Net VM bundle gate

The daemon refuses to start a `sys-<env>-net` VM (one of
`sys-corp-net`, `sys-personal-net`, `sys-obs-net`, …) when the
on-disk dnsmasq.conf hash for that env diverges from the hash the
trusted bundle implies. This catches the case where the bundle was
updated but the dnsmasq render step — a host singleton or systemd
unit — never reran, so the running net VM would silently serve a
stale lease table to its workloads.

> **Current ordering note.** The preflight still runs from
> `dispatch_broker_vm_start` before the rest of the start path.
> `HashMismatch`, `ConfigReadFailed`, and `EnvMissing` remain
> fail-closed. `ConfigMissing` is handled differently: the daemon
> logs a warning and soft-defers that case so a fresh install can
> continue until the daemon-owned dnsmasq render step is used end to
> end.

## Trust boundary

| Property | Value |
| --- | --- |
| Runs in | `nixlingd` (unprivileged) |
| Invoked from | `dispatch_broker_vm_start`, before the host-prep DAG executes. `ConfigMissing` is soft-deferred with a warning; the other drift classes are fail-closed. |
| Subject | `${dnsmasq_dir}/<env>.conf` (default `/var/lib/nixling/dnsmasq/<env>.conf`) |
| Capabilities used | — (pure `read()` against a file the daemon already has read access to) |
| Failure mode | `HashMismatch`, `ConfigReadFailed`, and `EnvMissing` refuse VM start with typed `daemon.bundle-dnsmasq-drift` (exit code `63`); `ConfigMissing` soft-defers with a warning |
| Scope | net VMs (`is_net_vm = true` in `vms.json`); workload VMs short-circuit |

The check is a pure SHA-256 comparison; the daemon does not attempt
to repair drift in the `HashMismatch` case (operator-driven recovery
per [Recovery](#recovery) below). `ConfigMissing` is currently a
warning-only soft-defer so a fresh host can proceed without turning
that first start into a hard failure.

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

## Drift classes

| Drift variant | Refusal envelope `message` (after redaction) | Operator action |
| --- | --- | --- |
| `EnvMissing` | "net VM '<vm>' has no env in manifest" | Fix `vms.json`; rebuild and reactivate the bundle. |
| `ConfigMissing` | "dnsmasq.conf for env '<env>' is missing; bundle/dnsmasq render did not run" | Warning-only soft-defer on the current start path so a fresh host can continue. If the file should already exist, regenerate it with `nixos-rebuild switch` or `nixling host prepare --apply` and retry. |
| `ConfigReadFailed` | "dnsmasq.conf for env '<env>' could not be read: <errno detail>" | Restore the file's ownership/mode (it should be daemon-readable). |
| `HashMismatch` | "dnsmasq.conf hash for env '<env>' diverges from bundle expectation (expected <sha256>, actual <sha256>); rebuild required" | The bundle was updated but the dnsmasq render step did not rerun. Re-render dnsmasq.conf and retry. |

All four are surfaced as the single typed-error variant
`TypedError::BundleDnsmasqDrift` with exit code `63` and kind
`bundle-dnsmasq-drift`. The full unredacted path is logged at
`warn!` level so operators can debug from `journalctl -u
nixlingd.service`; the public envelope intentionally omits it.

## Ordering

The preflight runs in `dispatch_broker_vm_start` after bundle
resolver load and before:

* the host-prep DAG resolution (`build_host_prep_dag`) is logged;
* the per-VM ownership-matrix and ssh-host-key preflights run; and
* any broker mutating op fires.

That ordering is intentional: `HashMismatch`, `ConfigReadFailed`,
and `EnvMissing` surface before any host mutation is attempted on
behalf of the stale net VM. Workload VMs (`is_net_vm = false`)
short-circuit to `NotANetVm` with zero filesystem reads.

`ConfigMissing` is the one exception. The daemon logs a warning and
continues the start path instead of turning a missing
`/var/lib/nixling/dnsmasq/<env>.conf` into a hard refusal.

## Recovery

The canonical recovery is to re-render the dnsmasq config and retry.
A full `nixos-rebuild switch` is sufficient, and `nixling host
prepare --apply` is the focused recovery path when you only need to
refresh the daemon-owned host-prep state. After the refresh:

```bash
ls -l /var/lib/nixling/dnsmasq/<env>.conf
sha256sum /var/lib/nixling/dnsmasq/<env>.conf
nixling vm start sys-<env>-net --apply
```

If the file is merely missing on a fresh host, the current start path
logs a warning and continues. If the file exists but still mismatches,
the typed envelope remains `daemon.bundle-dnsmasq-drift` (exit code
`63`) until the on-disk config matches the bundle again.

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
  the other preflight gates.

## Cross-references

* [`docs/reference/privileges.md`](./privileges.md) — daemon-side
  VM-start preflight catalog.
* [`packages/nixlingd/src/ssh_host_key_preflight.rs`](../../packages/nixlingd/src/ssh_host_key_preflight.rs) —
  sibling preflight (same trust boundary, different subject).
