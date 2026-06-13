# `nixling.daemonExperimental.enable` — v1.0 status

## v1.0 (ADR 0015): on by default

In v1.0 the daemon-only end-state landed: every mutating verb dispatches
through `nixlingd` → `nixling-priv-broker`, and the historical bash CLI
(`nixos-modules/cli.nix`) and the three-mode bridge (`default
daemon-first` / `NIXLING_NATIVE_ONLY=1` /
`NIXLING_LEGACY_BASH_OPT_IN=1`) were retired in v1.0.

`nixling.daemonExperimental.enable` is therefore a **legacy toggle** that
v1.0 leaves as the default-on shape required for the daemon, broker
socket, and bundle-artifact files to exist. Disabling it on v1.0 leaves
the host without an operator path; `nixling vm start --apply` will fail
with `daemon-down` (exit 1).

Enabling it adds the v1.0 daemon surface to the host:

- the `nixlingd` system user/group;
- the `nixlingd.service` unit plus the public/private sockets;
- the `nixling-priv-broker.{service,socket}` units (socket-activated;
  see ADR 0015);
- the Rust CLI + manpages/completions;
- `/etc/nixling/{bundle,host,processes,privileges}.json` emitted at
  rebuild time so the daemon + broker can resolve VM intents.

## What the daemon dispatches in v1.0

The daemon dispatches every mutating verb through the broker socket:

- VM lifecycle: `vm start / stop / restart / list` via broker
  `SpawnRunner` / `SignalRunner` + supervisor DAG (per-share virtiofsd,
  cloud-hypervisor, swtpm-flush + long-lived swtpm, vsock-relay, audio,
  GPU, video, USBIP sidecars).
- Host reconcile: `host install` via broker `RunHostInstall` (wired
  live). `host prepare / destroy --apply` are **not yet wired** — they
  return `daemon-down` (exit 1) today; use `--dry-run` for now. Their
  broker reconcile-op dispatch (`ApplyNftables` / `ApplyRoute` /
  `ApplySysctl` / `UpdateHostsFile` / `ApplyNmUnmanaged`) is
  forthcoming when daemon-side dispatch ships.
- Activation: `switch / boot / test / rollback / gc` via broker
  `RunActivation` / `RunGc`.
- Key lifecycle: `trust / rotate-known-host / keys rotate` via broker
  `RunHostKeyTrust` / `RunRotateKnownHost` / `RunKeysRotate`.
- USBIP: `usb attach / detach / probe` via broker SpawnRunner.
- Migration: `migrate` via broker `RunMigrate`.

Read-only verbs (`list`, `status`, `audit`, `host check`, `auth status`)
still answer directly from `nixlingd` over the public socket.

## Rollback

Roll back by reverting the host generation and rebuilding. There is no
env-var escape hatch in v1.0; the `NIXLING_LEGACY_BASH_OPT_IN=1` /
`NIXLING_NATIVE_ONLY=1` knobs from the three-mode bridge are no-ops
(they were removed in v1.0 under ADR 0015).

## See also

- [ADR 0015 — daemon-only clean-break](../adr/0015-daemon-only-clean-break.md)
- [v0 → v1 migration guide](../how-to/migrate-nixling-v0-to-v1.md)
- [CHANGELOG 1.0.0](../../CHANGELOG.md)
