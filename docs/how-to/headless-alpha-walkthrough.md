# Headless alpha v1.0 walkthrough

This how-to walks a clean Ubuntu 24.04 host from "no nixling
installed" to "a running headless Cloud Hypervisor VM on the v1.0
daemon/broker control-plane path (ADR 0015 daemon-only — the
historical three-mode bridge was retired in v1.0)".

**Status:** the wire + pure layer is still the foundation, but
the broker/runtime story has moved on. The production
(non-bootstrap) broker dispatcher now has live handlers for
`ApplyNftables`, `ApplyRoute`, `ApplySysctl`, `UpdateHostsFile`,
`OpenPidfd`, and `SpawnRunner`.
Operator-facing mutating verbs in v1.0 dispatch through `nixlingd` →
`nixling-priv-broker` only (ADR 0015 daemon-only). The three-mode
bridge (default daemon-first / `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1`)
was retired in v1.0 along with the bash CLI itself; both env vars are
now no-ops. See the
[v1.0 daemon-only ADR](../adr/0015-daemon-only-clean-break.md) for
the full removal list.

## Prerequisites

- Ubuntu 24.04 (24.10/25.04 should also work; older releases are
  not supported per the platform matrix).
- A kernel ≥ 6.6 with KVM enabled (`/dev/kvm` present, your user in
  the `kvm` group).
- `nix` installed via the upstream installer:
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh -s -- install
  ```
- Flake support enabled (the Determinate installer turns this on by
  default; otherwise add `experimental-features = nix-command flakes`
  to `~/.config/nix/nix.conf`).

## Step 1 — Install nixling

```bash
nix profile install github:vicondoa/nixling#nixling
```

The profile install pulls in the `nixling` CLI binary, the
`nixlingd` daemon, the privileged broker, and the systemd unit
templates. `nixling host install --apply` now drives the live
installer through the daemon → broker path; this alpha walkthrough
keeps the manual steps so operators can see exactly what the
installer materializes:

```bash
sudo mkdir -p /etc/nixling
sudo cp -r ~/.nix-profile/share/nixling/units/* /etc/systemd/system/
sudo cp ~/.nix-profile/share/nixling/daemon-config.json /etc/nixling/
sudo systemctl daemon-reload
```

## Step 2 — Define a VM

Place a minimal manifest under `/etc/nixling/bundle.json` describing
one headless VM. The bundle ships with the `examples/minimal`
template:

```bash
sudo cp ~/.nix-profile/share/nixling/examples/minimal/bundle.json /etc/nixling/
sudo cp ~/.nix-profile/share/nixling/examples/minimal/host.json /etc/nixling/
sudo cp ~/.nix-profile/share/nixling/examples/minimal/processes.json /etc/nixling/
sudo cp ~/.nix-profile/share/nixling/examples/minimal/vms.json \
        /run/current-system/sw/share/nixling/vms.json
```

The example declares one VM (`corp-vm`) in the `work` env with one
TAP interface, four virtiofs shares (`ro-store`, `nl-meta`,
`nl-hkeys`, `nl-ssh-host`), and no TPM / GPU / observability roles.

## Step 3 — Prepare the host

```bash
nixling host prepare --dry-run
nixling host prepare --apply
```

`host prepare` reconciles host-shared state (cgroup delegation, the
named `inet nixling` nft table, NetworkManager unmanaged drop-in,
`/etc/hosts` managed block, sysctl ordering, kernel module probe).
The dry-run path is complete today. In the production broker
dispatcher, the host-reconcile ops behind `ApplyNftables` /
`ApplyRoute` / `ApplySysctl` / `UpdateHostsFile` are now live; the
remaining rollout work is the public daemon-backed `host prepare
--apply` surface, not the broker executor itself.

## Step 4 — Inspect the DAG

```bash
nixling vm start corp-vm --dry-run --json
```

Output: the 5-node DAG the supervisor models today. The DAG shape
is stable; `--apply` routes through the daemon-native dispatch
(v1.0 daemon-only per ADR 0015). This returns:

```jsonc
{
  "command": "vm start",
  "mode":    "dry-run",
  "vm":      "corp-vm",
  "dag": {
    "nodes": [
      {"id": "host-reconcile",      "role": "host-reconcile"},
      {"id": "store-preflight",     "role": "store-virtiofs-preflight"},
      {"id": "virtiofsd-ro-store",  "role": "virtiofsd"},
      {"id": "ch",                  "role": "cloud-hypervisor-runner"},
      {"id": "guest-control-health", "role": "guest-control-health"}
    ],
    "edges": [
      {"from": "host-reconcile",     "to": "store-preflight"},
      {"from": "store-preflight",    "to": "virtiofsd-ro-store"},
      {"from": "virtiofsd-ro-store", "to": "ch"},
      {"from": "ch",                 "to": "guest-control-health"}
    ]
  },
  "notes": "vm dry-run reports the DAG the supervisor would drive; --apply routes through the daemon-native dispatch (v1.0 daemon-only per ADR 0015)."
}
```

## Step 5 — Start the VM

```bash
sudo systemctl start nixlingd.service
nixling vm start corp-vm --apply
```

The native DAG is still the same 5-node sequence, but the behavior
is different from the original draft:

1. `host-reconcile` — the production broker dispatcher now resolves
   bundle intent refs and runs live `ApplyNftables` / `ApplyRoute` /
   `ApplySysctl` / `UpdateHostsFile` handlers.
2. `store-preflight` — the same runner-shape preflight still guards
   the virtiofs / runner surface before launch.
3. `virtiofsd-ro-store` + `ch` — the broker's non-bootstrap
   `SpawnRunner` handler is live and returns pidfds over SCM_RIGHTS;
   the daemon can re-open / re-adopt them through the live
   `OpenPidfd` path.
4. `guest-control-health` — the daemon runs the authenticated
   guest-control Health probe (Hello + token challenge-response +
   Health over the guest-control vsock) on guest-control-capable VMs.
   It fails closed and is the guest-readiness gate; SSH is a compat
   surface only, so the legacy raw TCP-22 `ssh-ready` /
   `guest-ssh-readiness` node was removed and is no longer emitted.

The operator-facing routing changed: `nixling vm start corp-vm --apply`
no longer stops at the old `daemon-down` placeholder by default.
Instead:

In v1.0 daemon-only (ADR 0015) there is exactly one routing path:
`--apply` dispatches through `nixlingd` → `nixling-priv-broker`.
Daemon-unreachable surfaces the typed `daemon-down` envelope (exit-1);
native-handler-deferred surfaces `not-yet-implemented` (exit-78).
The historical `NIXLING_NATIVE_ONLY=1` and `NIXLING_LEGACY_BASH_OPT_IN=1`
env vars are no-ops; the bash CLI itself was retired in v1.0.

## Step 6 — Observe runtime state

```bash
nixling vm list --json
nixling status corp-vm
nixling audit
```

`vm list` returns the daemon's runtime view. If `vm start --apply`
the daemon path is unavailable (returns exit-78 per ADR 0015), that view can still be empty today even though the
VM is up; once the native-only path owns the lifecycle end-to-end,
`vm list` will populate from daemon state. `status corp-vm` returns
the per-VM manifest + service view including any `[pending restart]`
annotation. `audit` streams the broker's append-only audit log
(`/var/lib/nixling/audit/broker-<utc-date>.jsonl`).

## Step 7 — Stop / restart (v1.0 daemon-only routing)

```bash
nixling vm stop corp-vm --apply
nixling vm restart corp-vm --apply
```

The same v1.0 daemon-only routing applies here (ADR 0015): `stop`
dispatches through `nixlingd` → broker `SignalRunner`. Native `stop`
walks the DAG in reverse topo order, signalling each pidfd with
`pidfd_send_signal(SIGTERM)` and waiting for `waitid(P_PIDFD)`.
`restart` is `stop` then `start`. The `NIXLING_NATIVE_ONLY=1` and
`NIXLING_LEGACY_BASH_OPT_IN=1` env vars from the three-mode bridge
are no-ops in v1.0; the bash CLI itself was retired in v1.0.

## Reference shape — what's live today

| Component                 | Wire-stable | Live today | Remaining rollout |
|---------------------------|:-----------:|:----------:|:-----------------:|
| `ch_argv` generator       | ✅                  | ✅         | — |
| virtiofsd argv / shares   | ✅                  | ✅         | emitted by `nixos-modules/processes-json.nix`; see `docs/reference/store-virtiofs.md` |
| `swtpm_argv` generator    | ✅                  | ✅ (opt-in)| — |
| Supervisor DAG executor   | ✅                  | ✅ (pure)  | native-only end-to-end ownership |
| Broker host-reconcile ops (`ApplyNftables` / `ApplyRoute` / `ApplySysctl` / `UpdateHostsFile`) | ✅ | ✅ (non-bootstrap dispatch) | public `host prepare --apply` rollout |
| Broker `OpenPidfd` op     | ✅                  | ✅ (non-bootstrap dispatch) | broader operator surfacing |
| Broker `SpawnRunner` op   | ✅                  | ✅ (non-bootstrap dispatch) | full native-only CLI rollout |
| Daemon state persistence  | ✅                  | ✅ (pure)  | native-only end-to-end ownership |
| Daemon `[pending restart]`| ✅                  | ✅         | — |
| `nixling vm` CLI verbs    | ✅                  | ✅ (`--dry-run`; `--apply` uses daemon-only routing) | native-only lifecycle rollout |
| Ubuntu Tier-1 smoke       | ✅ (docs)           | —          | repeated live-host green runs |

The wire-stable column means the JSON/argv shape and the typed
envelope shape are pinned today; future changes follow the
wire-skew contract (`PROTOCOL_VERSION` bump + version-skew gate).

## References

- [Daemon lifecycle explanation](../explanation/daemon-lifecycle.md)
- [Runner-shape audit](../reference/runner-shape-audit.md) —
  the parity oracle the generator matches.
- [Daemon API reference](../reference/daemon-api.md)
- [Error codes](../reference/error-codes.md) — the typed envelope
  catalog including `daemon-down` / `not-yet-implemented` /
  `--apply-or-dry-run-required` used by mutating verbs.
