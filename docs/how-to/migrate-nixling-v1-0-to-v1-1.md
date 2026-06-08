# Migrating from nixling v1.0 to v1.1

This guide covers the operator-visible changes between nixling
v1.0 (released 2025-Q4) and v1.1 (released 2026-Q2). v1.1 is the
"daemon-only clean break" follow-through: every v1.0 deferral
listed under CHANGELOG § "Deferred to follow-up commits" is closed,
and several latent v0.x compatibility shims are removed.

## Prerequisites

Before upgrading from v1.0 to v1.1, satisfy these prerequisites in
order:

1. **Linux kernel ≥ 6.9** (hard upgrade blocker). Operators on
   kernel 6.6–6.8 cannot run v1.1. The daemon's pidfs runtime
   self-probe in `packages/nixlingd/src/startup.rs` and the
   static eval gate in `tests/v1.1-kernel-floor-eval.sh`
   (introduced alongside v1.1-P10's broker reaping model) both
   require pidfs support, which landed in mainline 6.9. See
   [ADR 0008 § "v1.1 kernel-floor uplift"](../adr/0008-supported-platforms-and-rejected-targets.md)
   and [ADR 0018 § "set-booted race-free serialization"](../adr/0018-microvm-nix-removal.md#set-booted-race-free-serialization)
   for the rationale.

2. **Remove `nixling.daemonExperimental.enable`** from the
   consumer flake (or set it `false` — but `remove` is the
   canonical instruction). Leaving the option set in v1.1+
   emits an eval-time deprecation warning via the v1.1-P4
   assertion in `nixos-modules/assertions.nix`. The warning
   text — emitted verbatim by `nixos-rebuild` AND locked into
   this guide — is:

   > `nixling.daemonExperimental.enable` is obsolete in v1.1;
   > remove this option from your consumer flake because the
   > broker socket/service are enabled by default. Leaving it
   > set has no effect.

3. **Optional**: snapshot `/etc/nixos` / consumer-flake state
   before the upgrade in case you need to roll back.

## What changed (operator-visible)

### v1.1-P1 — Bash fallback removed

`exec_legacy_passthrough` and `should_fallback_to_legacy` were
deleted from `packages/nixling/src/lib.rs`. The Rust CLI never
executes bash in v1.1+; verbs whose daemon-native handler is not
yet implemented surface typed envelopes instead:

| Verb                | v1.0 behaviour                                                                | v1.1 behaviour                                                                 |
| ------------------- | ----------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| `nixling audit --strict` | Returned exit-78 envelope via the bash-fallback message helper        | Returns typed `not-yet-implemented` envelope (exit 78) directly                |
| `nixling audit` (daemon-unreachable) | Returned exit-78 envelope via the bash-fallback message helper | Returns typed `daemon-down` envelope (exit 1) directly                         |
| `nixling console`   | Returned exit-78 envelope via the bash-fallback message helper                | Returns typed `not-yet-implemented` envelope (exit 78) directly                |
| `nixling audio`     | Returned exit-78 envelope via the bash-fallback message helper                | Returns typed `not-yet-implemented` envelope (exit 78) directly                |
| `nixling keys list` (daemon-unreachable) | Returned exit-78 envelope                          | Returns typed `daemon-down` envelope (exit 1) directly                         |
| `nixling keys show` (daemon-unreachable) | Returned exit-78 envelope                          | Returns typed `daemon-down` envelope (exit 1) directly                         |

Removed environment variables (no-op since v1.0, removed entirely in v1.1):
- `NIXLING_LEGACY_BASH_OPT_IN`
- `NIXLING_LEGACY_CLI`
- `NIXLING_LEGACY_CLI_PATH`

`NIXLING_NATIVE_ONLY` is retained as a no-op (its semantics are
now the default) for one more release; future minor releases may
drop the no-op too.

New eval gate: `tests/no-bash-exec-eval.sh` (3 modes: `check`,
`fixture-coverage`, `syn-ast-walk`). Allow-list is
`tests/fixtures/no-bash-exec-exempt-paths.json` (empty at v1.1
landing time).

### v1.1-P2 — `nixling.vms.<vm>.supervisor` option removed

The `supervisor` per-VM option was removed from
`nixos-modules/options-vms.nix`. Setting it in a consumer flake
fails eval with this typed friendly error (via the per-submodule
`mkRemovedOptionModule` shim in
`nixos-modules/options-vms-removed.nix`):

> `nixling.vms.<vm>.supervisor` was removed in v1.1 per ADR 0015
> (daemon-only clean break). The v1.0 daemon-only end-state makes
> `"nixlingd"` the only valid supervisor; v1.1 completes the
> migration by deleting the option entirely.
>
> Migration: remove every `supervisor = ...` line from your
> consumer flake's `nixling.vms.<vm>.*` declarations. The
> daemon-only path is the default and only path.

**Action**: remove every `supervisor = "..."` line from your
consumer flake before upgrading.

If your v1.0 deployment was mixed (some VMs `supervisor =
"systemd"`, others `supervisor = "nixlingd"`), the v1.1 default
is daemon-supervised for all enabled VMs. If you previously relied
on the `"systemd"` template path, the v1.1 broker SpawnRunner
pipeline (already shipped in v1.0 as the daemon-only path) is the
canonical replacement.

### v1.1-P3 — Bundle resolver runner-intent regression coverage

A focused integration test
(`packages/nixling-core/tests/bundle_resolver_runner_intents.rs`)
guards against the `internal-io` envelope failure class seen during
the v1.0 closeout side-task. No operator-visible behaviour change.

### v1.1-P4 — Broker NixOS module default-on

`nixos-modules/host-broker.nix` no longer gates its config block
behind `cfg.daemonExperimental.enable`. Enabling the nixling host
module brings `nixling-priv-broker.service` +
`nixling-priv-broker.socket` up automatically; no operator opt-in
to an experimental flag.

**Action**: after upgrading, verify the broker socket activates
cleanly:

```bash
systemctl is-active nixling-priv-broker.service
systemctl status nixling-priv-broker.socket
```

If the broker fails to activate, the typical causes are:
1. `/run/nixling/priv.sock` already exists with wrong ownership
   (carry-over from a manual `start-nixling-vms.sh` workaround in
   v1.0). Delete the socket file, then `systemctl restart
   nixling-priv-broker.socket`.
2. Stale daemon state in `/var/lib/nixling/runtime/`. Restart
   `nixlingd nixling-priv-broker.service nixling-priv-broker.socket`
   in that order.

### v1.1-P5 — `/var/lib/nixling` permission tightening

The parent state directory is `0750 root:nixlingd` (was the same
in v1.0). v1.1 adds a `nixlingStateDirAcl` activation script that
grants per-sidecar-user `--x` (traversal-only) ACLs on
`/var/lib/nixling`. Without these grants, sidecar users not in
the `nixlingd` group could not reach their per-VM subdirectories.

**Action**: after `nixos-rebuild switch`, verify:

```bash
stat -c '%a %U %G' /var/lib/nixling          # expect: 750 root nixlingd
getfacl /var/lib/nixling | head -20          # expect per-sidecar user:nixling-<vm>-{gpu,swtpm,audio,video}:--x entries
```

### v1.1-P6 — OTel host-bridge moved to broker SpawnRunner

`nixos-modules/host-otel-relay-acl.nix` is no longer imported via
`nixos-modules/default.nix`. The OTel host-bridge ACL contract
migrated into the broker pre-spawn pipeline
(`RunnerRole::OtelHostBridge` in
`packages/nixling-ipc/src/broker_wire.rs`, handler in
`packages/nixling-priv-broker/src/runtime.rs`).

No operator-visible change if your v1.0 deployment used the
`nixling-otel-host-bridge.service` host singleton — the broker
SpawnRunner is the v1.1 replacement and is wired identically.

### v1.1-P7 — `nixling-vfsd-watchdog@.{service,timer}` retired

The per-VM watchdog systemd template + timer are removed from
`nixos-modules/store.nix`. Wedge detection moves into the broker's
Virtiofsd `SpawnRunner` role supervisor (pidfd-based, same ~60s
cadence as the retired timer).

**Action**: no operator action required. Wedge events now surface
via `nixling audit` and the broker journal:

```bash
nixling audit | grep runner-wedged
journalctl -u nixling-priv-broker.service | grep runner-wedged
```

### v1.1-P8..P11 — Substrate replacement COMPLETE

The substrate-replacement work shipped in v1.1-final: per-VM reads
re-homed from `config.microvm.vms.<vm>.config.config.microvm.*`
to nixling-owned helpers (`nl.vmRunner` / `nl.vmToplevel` /
`nl.vmDeclaredRunner` in `nixos-modules/lib.nix`), the
nixling-owned per-VM evaluator
(`nixos-modules/vm-evaluator.nix` + `vm-options.nix`) replaces
microvm.nix's host-module evaluation, and **`inputs.microvm` is
removed from `flake.nix`** entirely. All 13 v1.1 invariant gates
(including `microvm-nix-absent-eval.sh`) PASS.

**Action — consumer flake update**: if your consumer flake's
`flake.nix` declares its own `inputs.microvm`, drop it. Nixling
no longer depends on the upstream microvm.nix flake, so your
consumer's lock file should not pin a `microvm` entry on
nixling's behalf. Run `nix flake update` (or `nix flake lock
--update-input microvm`) after dropping the input to regenerate
your lock.

If you previously used microvm.nix directly in your own VM
declarations (outside the nixling framework), you can keep your
own `inputs.microvm` for those use cases — nixling no longer
imports the host module, but nothing prevents you from importing
it yourself for non-nixling VMs.

### v1.1-P12 — Docs polish

This guide, the updated ADR statuses (0015, 0017, 0018), the
CHANGELOG "Retired from v1.0 deferral list" section, and the
tagline sweep (drop "on microvm.nix" from `flake.nix` /
`README.md` / `AGENTS.md` taglines) all land in v1.1-P12.

## `nixling status` output schema (v1.0 vs v1.1 vs v1.1.1)

> **v1.1.1 status note**: v1.1.1 ships the `StatusOutputV3` wire
> schema (`packages/nixling/src/lib.rs` `StatusServicesOutputV3`
> + `from_v2` migration shim) per the rename map below. The CLI
> `nixling status` command still EMITS the v1.0/v1.1
> `StatusServicesOutputV2` shape at v1.1.1; the emit-side
> flip to V3 is scheduled for v1.1.2.
>
> Tooling authors that consume the JSON output should:
> - At v1.1.1, continue parsing V2 (`microvm`/`snd`/`virtiofsd`).
> - At v1.1.2+, parse V3 (`hypervisor`/`audio`/`virtiofsd_per_share`/...)
>   with the documented rename map below.
> - The `StatusServicesOutputV3::from_v2()` migration shim lives
>   in the public surface so tooling can adopt incrementally.

### v1.1.1 SHIPPED → CLI-emit at v1.1.2 rename map

**Bracketed names** in the V3 schema identify per-resource
instances: `virtiofsd[store]` is the share whose `tag` is `store`;
`usbip_backend[default]` is the USBIP backend for the env named
`default`. The bracketed convention is PROSE in human-form
output; JSON uses `{"virtiofsd_per_share": {"store": {...}},
"usbip_backend_per_env": {"default": {...}}}`.

| V2 field (current CLI output)    | V3 field (wire-side, v1.1.1+) | Notes                                                              |
| -------------------------------- | ----------------------------- | ------------------------------------------------------------------ |
| `nixling`                        | (deleted)                     | The pre-P6 wrapper unit was removed in v1.0; V3 drops the field.   |
| `microvm`                        | `hypervisor`                  | Cloud Hypervisor runner is broker-spawned in v1.1.                 |
| `virtiofsd`                      | `virtiofsd_per_share[<tag>]`  | Per-share entry instead of a single field.                         |
| `gpu`                            | `gpu`                         | Unchanged name; broker-spawned in v1.1.                            |
| `snd`                            | `audio`                       | Renamed to match the role-catalog naming convention.               |
| `swtpm`                          | `swtpm`                       | Unchanged name; broker-spawned in v1.1.                            |
| (no V2 field)                    | `otel_relay`                  | New per-VM field — broker-spawned OtelGuestRelay per ADR 0018.     |
| (no V2 field)                    | `otel_host_bridge`            | New host-scoped field — broker-spawned OtelHostBridge.             |
| (no V2 field)                    | `usbip_backend_per_env[<env>]`| New host-scoped field — broker-spawned USBIP backend per env.      |
| (no V2 field)                    | `usbip_proxy_per_env[<env>]`  | New host-scoped field — broker-spawned USBIP proxy per env.        |

## Recovery — broker bring-up troubleshooting

If `nixling vm start --apply <vm>` returns a `daemon-down` envelope
after upgrading:

```bash
# 1. Verify the broker socket + service are active
systemctl status nixling-priv-broker.socket
systemctl status nixling-priv-broker.service

# 2. If inactive, restart in order
systemctl restart nixling-priv-broker.socket nixling-priv-broker.service nixlingd

# 3. Inspect the broker journal for activation errors
journalctl -u nixling-priv-broker.service | head -50

# 4. Re-attempt the verb
nixling vm start --apply <vm>
```

If the broker journal shows a peer-cred or ACL error on the
private socket, verify the daemon is running as `nixlingd` (not
root) and that the socket is `0660 root:nixlingd`:

```bash
ls -lZ /run/nixling/priv.sock
# Expected: 0660 root:nixlingd
```

## See also

- [ADR 0015 — Daemon-only clean break](../adr/0015-daemon-only-clean-break.md) — overall v1.0 → v1.1 narrative.
- [ADR 0017 — No bash fallbacks invariant](../adr/0017-no-bash-fallbacks-invariant.md) — v1.1-P1 rationale.
- [ADR 0018 — Removal of the microvm.nix flake dependency](../adr/0018-microvm-nix-removal.md) — v1.1-P6..P11 rationale and roadmap.
- [`docs/reference/cli-contract.md`](../reference/cli-contract.md) — per-verb invariants in v1.1+.
- [`docs/reference/error-codes.md`](../reference/error-codes.md) "Remediation rendering conventions" — typed-envelope format.
- [`docs/reference/privileges.md`](../reference/privileges.md) — broker capability matrix + per-role ACL contract.
