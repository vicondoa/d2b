# Headless alpha v1.0 walkthrough

This how-to walks a clean Ubuntu 24.04 host from "no d2b
installed" to "a running headless Cloud Hypervisor VM on the v1.0
daemon/broker control-plane path (ADR 0015 daemon-only - the
historical three-mode bridge was retired in v1.0)".

**Status:** the wire + pure layer is still the foundation, but
the broker/runtime story has moved on. The production
(non-bootstrap) broker dispatcher now has live handlers for
`ApplyNftables`, `ApplyRoute`, `ApplySysctl`, `UpdateHostsFile`,
`OpenPidfd`, and `SpawnRunner`.
Operator-facing mutating verbs in v1.0 dispatch through `d2bd` →
`d2b-priv-broker` only (ADR 0015 daemon-only). The three-mode
bridge (default daemon-first / `D2B_NATIVE_ONLY=1` / `D2B_LEGACY_BASH_OPT_IN=1`)
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

## Step 1 - Install d2b

```bash
nix profile install github:vicondoa/d2b#d2b
```

The profile install pulls in the `d2b` CLI binary, the
`d2bd` daemon, the privileged broker, and the systemd unit
templates. `d2b host install --apply` now drives the live
installer through the daemon → broker path; this alpha walkthrough
keeps the manual steps so operators can see exactly what the
installer materializes:

```bash
sudo mkdir -p /etc/d2b
sudo cp -r ~/.nix-profile/share/d2b/units/* /etc/systemd/system/
sudo cp ~/.nix-profile/share/d2b/daemon-config.json /etc/d2b/
sudo systemctl daemon-reload
```

## Step 2 - Define a VM

Place a minimal manifest under `/etc/d2b/bundle.json` describing
one headless VM. The bundle ships with the `examples/minimal`
template:

```bash
sudo cp ~/.nix-profile/share/d2b/examples/minimal/bundle.json /etc/d2b/
sudo cp ~/.nix-profile/share/d2b/examples/minimal/host.json /etc/d2b/
sudo cp ~/.nix-profile/share/d2b/examples/minimal/processes.json /etc/d2b/
sudo cp ~/.nix-profile/share/d2b/examples/minimal/vms.json \
        /run/current-system/sw/share/d2b/vms.json
```

The example declares one VM (`corp-vm`) in the `work` env with one
TAP interface, four virtiofs shares (`ro-store`, `d2b-meta`,
`d2b-hkeys`, `d2b-ssh-host`), and no TPM / GPU / observability roles.

## Step 3 - Prepare the host

```bash
d2b host prepare --dry-run
# `--apply` is not yet wired: it returns the typed `daemon-down`
# envelope (exit 1) today - use `--dry-run` for now.
d2b host prepare --apply
```

`host prepare` reconciles host-shared state (cgroup delegation, the
named `inet d2b` nft table, NetworkManager unmanaged drop-in,
`/etc/hosts` managed block, sysctl ordering, kernel module probe).
The dry-run path is complete today. The host-reconcile ops behind
`ApplyNftables` / `ApplyRoute` / `ApplySysctl` / `UpdateHostsFile` are
staged in the broker executor; the remaining rollout work is the
public daemon-backed `host prepare --apply` surface that dispatches
them, so `--apply` returns `daemon-down` (exit 1) until that ships.

## Step 4 - Inspect the DAG

```bash
d2b guest start corp-vm --dry-run --json
```

Output: the 5-node DAG the supervisor models today. The DAG shape
is stable; `--apply` routes through the daemon-native dispatch
(v1.0 daemon-only per ADR 0015). This returns:

```jsonc
{
  "command": "guest start",
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

## Step 5 - Start the VM

```bash
sudo systemctl start d2bd.service
d2b guest start corp-vm --apply
```

The native DAG is still the same 5-node sequence, but the behavior
is different from the original draft:

1. `host-reconcile` - the production broker dispatcher now resolves
   bundle intent refs and runs live `ApplyNftables` / `ApplyRoute` /
   `ApplySysctl` / `UpdateHostsFile` handlers.
2. `store-preflight` - the same runner-shape preflight still guards
   the virtiofs / runner surface before launch.
3. `virtiofsd-ro-store` + `ch` - the broker's non-bootstrap
   `SpawnRunner` handler is live and returns pidfds over SCM_RIGHTS;
   the daemon can re-open / re-adopt them through the live
   `OpenPidfd` path.
4. `guest-control-health` - the daemon runs the authenticated
   guest-control Health probe (Hello + token challenge-response +
   Health over the guest-control vsock) on guest-control-capable VMs.
   It fails closed and is the guest-readiness gate; SSH is a compat
   surface only, so the legacy raw TCP-22 `ssh-ready` /
   `guest-ssh-readiness` node was removed and is no longer emitted.

The operator-facing routing changed: `d2b guest start corp-vm --apply`
no longer stops at the old `daemon-down` placeholder by default.
Instead:

In v1.0 daemon-only (ADR 0015) there is exactly one routing path:
`--apply` dispatches through `d2bd` → `d2b-priv-broker`.
Daemon-unreachable surfaces the typed `daemon-down` envelope (exit-1);
native-handler-deferred surfaces `not-yet-implemented` (exit-78).
The historical `D2B_NATIVE_ONLY=1` and `D2B_LEGACY_BASH_OPT_IN=1`
env vars are no-ops; the bash CLI itself was retired in v1.0.

## Step 6 - Observe runtime state

```bash
d2b guest list --json
d2b guest status corp-vm
d2b audit
```

`guest list` returns the Guest resource view. If `guest start --apply`
the daemon path is unavailable (returns exit-78 per ADR 0015), that view can still be empty today even though the
VM is up; once the native-only path owns the lifecycle end-to-end,
`guest list` will populate from the resource plane. `guest status corp-vm` returns
the Guest envelope including `status.update`
annotation. `audit` streams the broker's append-only audit log
(`/var/lib/d2b/audit/broker-<utc-date>.jsonl`).

## Step 7 - Stop / restart (v1.0 daemon-only routing)

```bash
d2b guest stop corp-vm --apply
d2b guest restart corp-vm --apply
```

The same v1.0 daemon-only routing applies here (ADR 0015): `stop`
dispatches through `d2bd` → broker `SignalRunner`. Native `stop`
walks the DAG in reverse topo order, signalling each pidfd with
`pidfd_send_signal(SIGTERM)` and waiting for `waitid(P_PIDFD)`.
`restart` is `stop` then `start`. The `D2B_NATIVE_ONLY=1` and
`D2B_LEGACY_BASH_OPT_IN=1` env vars from the three-mode bridge
are no-ops in v1.0; the bash CLI itself was retired in v1.0.

## Reference shape - what's live today

| Component                 | Wire-stable | Live today | Remaining rollout |
|---------------------------|:-----------:|:----------:|:-----------------:|
| `ch_argv` generator       | ✅                  | ✅         | - |
| virtiofsd argv / shares   | ✅                  | ✅         | emitted by `nixos-modules/processes-json.nix`; see `docs/reference/store-virtiofs.md` |
| `swtpm_argv` generator    | ✅                  | ✅ (opt-in)| - |
| Supervisor DAG executor   | ✅                  | ✅ (pure)  | native-only end-to-end ownership |
| Broker host-reconcile ops (`ApplyNftables` / `ApplyRoute` / `ApplySysctl` / `UpdateHostsFile`) | ✅ | ✅ (non-bootstrap dispatch) | public `host prepare --apply` rollout |
| Broker `OpenPidfd` op     | ✅                  | ✅ (non-bootstrap dispatch) | broader operator surfacing |
| Broker `SpawnRunner` op   | ✅                  | ✅ (non-bootstrap dispatch) | full native-only CLI rollout |
| Daemon state persistence  | ✅                  | ✅ (pure)  | native-only end-to-end ownership |
| Daemon `status.update`  | ✅                  | ✅         | - |
| `d2b guest` / `d2b exec` CLI verbs | ✅ | ✅ (typed ResourceRef parsing; lifecycle UpdateSpec is a later-wave blocker) | native-only lifecycle rollout |
| Ubuntu Tier-1 smoke       | ✅ (docs)           | -          | repeated live-host green runs |

The wire-stable column means the JSON/argv shape and the typed
envelope shape are pinned today; future changes follow the
wire-skew contract (`PROTOCOL_VERSION` bump + version-skew gate).

## References

- [Daemon lifecycle explanation](../explanation/daemon-lifecycle.md)
- [Runner-shape audit](../reference/runner-shape-audit.md) -
  the parity oracle the generator matches.
- [Daemon API reference](../reference/daemon-api.md)
- [Error codes](../reference/error-codes.md) - the typed envelope
  catalog including `daemon-down` / `not-yet-implemented` /
  `--apply-or-dry-run-required` used by mutating verbs.
