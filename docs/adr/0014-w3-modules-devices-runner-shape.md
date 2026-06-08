# 0014. W3 `kernel.modules_disabled=1` behavior, module probe order, CH net handoff selection, and runner-shape preflight

- Status: Accepted
- Date: 2026-06-08
- Wave: W3
- Plan slice: §"W3 kernel-module probe order", §"W3 Cloud Hypervisor net handoff probe", §"W3 runner-shape preflight"
- Companion ADRs: [ADR 0002](0002-non-root-daemon-and-privileged-broker.md), [ADR 0003](0003-minijail-provisioning-and-sandbox-interface.md), [ADR 0004](0004-cloud-hypervisor-runner-shape.md), [ADR 0005](0005-network-firewall-and-tap-model.md)

## Context

W3 ships the host-prepare control plane. Three load-bearing decisions
made by W3 scope **s4** need their own ADR because they reach across
later waves (W4 mount namespace + virtiofsd consumes the recorded CH
net-handoff mode; W5 runner supervision consumes the runner-shape
preflight; future minijail revisions consume the
`modules_disabled` posture).

The questions the ADR closes:

1. What does W3 do when a hardened host has set
   `/proc/sys/kernel/modules_disabled=1`?
2. In what order does the broker probe kernel-module availability?
3. How does W3 select the Cloud Hypervisor TAP handoff mode?
4. What is the runner-shape preflight contract, and what is the
   golden-fixture drift surface?

## Decision

### `kernel.modules_disabled=1` posture

When `/proc/sys/kernel/modules_disabled` reads `1`, W3 refuses to
start any VM that declares a `required` kernel module not detected as
built-in or already loaded. The host check surfaces
`host-modules-locked` with the per-tier remediation hint. The broker
**never** attempts `modprobe(8)` in this state — the read happens
before any backend call. There is no override knob: the only
remediations are (a) reboot without `modules_disabled=1`, or (b) ship
the module as built-in.

### Four-step module probe order

Before any `ModprobeIfAllowed` decision, the broker reads, in order:

1. `/proc/sys/kernel/modules_disabled` (locked / unlocked).
2. `/proc/modules` + `/sys/module/<name>/` (loaded set).
3. `/lib/modules/$(uname -r)/modules.builtin` (preferred) or
   `modules.builtin.bin` (built-in set).
4. `/boot/config-$(uname -r)` or `/proc/config.gz` (`CONFIG_*`
   secondary evidence only).

`ModprobeIfAllowed` accepts only module names that appear in the
trusted bundle's `kernelModules` matrix as `required` or `optional`
with `loadAllowed: true`. Everything else is refused with audit. Step 2
detection of `br_netfilter` surfaces a recommendation to pin
`net.bridge.bridge-nf-call-iptables=0` and
`net.bridge.bridge-nf-call-ip6tables=0`; suppression requires an
ADR opt-in.

### Cloud Hypervisor net handoff selection

W3 records the selected mode in `host.json` under
`host.ch.netHandoffMode`. Detection runs against the packaged
`ch --help` output:

- `tap-fd` (preferred) — broker opens TAP + `/dev/vhost-net` and
  passes file descriptors via `SCM_RIGHTS`; runner has no
  `CAP_NET_ADMIN`.
- `persistent-tap` (fallback) — broker creates a persistent TAP with
  `TUNSETOWNER` / `TUNSETGROUP` to the runner uid/gid; runner mounts
  the device node read-only.

If neither mode satisfies the declared VM network resources without
`CAP_NET_ADMIN`, W3 fails closed with `ch-net-handoff-not-supported`.
The selection is recorded once at host check time and is consumed by
W4.

### Runner-shape preflight

`nixling host check` runs a dry-run preflight that consumes
`host.json`, `processes.json`, and `closures/<vm>.json` without
launching CH. The preflight validates:

- packaged CH capability rows match the declared row set;
- every enabled VM has a `declaredRunner` argv-hash snapshot;
- CH API socket paths declare `mode = 0660` and a non-empty owner;
- vsock transports are Unix-socket-backed;
- virtiofsd / swtpm sidecars reference a node id present in
  `processes.json`'s DAG.

The golden fixture
[`tests/golden/runner-shape/parity-drift.json`](../../tests/golden/runner-shape/parity-drift.json)
encodes the failure path and is consumed by
`tests/runner-shape-preflight.sh`.

## Consequences

- Operators on hardened hosts that lock module loading at boot must
  ship the required modules built-in; this is the documented W3
  Tier-0 / Tier-1 stance.
- The `tap-fd` preference removes `CAP_NET_ADMIN` from the long-lived
  runner, which is the W3 sandbox baseline (ADR 0003 + ADR 0005).
- The runner-shape preflight catches `declaredRunner` drift before any
  CH binary spawn, preserving the W0b ADR 0004 oracle.
- Future waves (W4 mount namespace, W5 runner supervision) consume the
  recorded CH net-handoff mode and the runner-shape findings without
  re-probing; reprobing on every VM start would re-introduce
  TOCTOU between host-check and start.
