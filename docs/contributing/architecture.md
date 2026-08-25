# Architecture conventions

Naming rules the framework enforces at eval time, what belongs in this repo
versus a sibling flake, and how the daemon-supervised VM lifecycle is shaped.

The binding summary is in [`../../AGENTS.md`](../../AGENTS.md). The daemon-only
end-state is recorded in
[`../adr/0015-daemon-only-clean-break.md`](../adr/0015-daemon-only-clean-break.md),
which is authoritative if this file drifts.

## Naming conventions

The framework declares **exactly three** root-visible units. There
is no `d2b@<vm>`-style per-VM unit; `d2bd` supervises every
per-VM DAG in-process and hands fds to spawned runners via the
broker's `SpawnRunner` / `OpenPidfd` ops.

| Resource                                | Pattern                                |
| --------------------------------------- | -------------------------------------- |
| Public daemon (supervisor)              | `d2bd.service`                     |
| Privileged broker socket                | `d2b-broker.socket`           |
| Privileged broker service               | `d2b-broker.service`          |
| Lifecycle permission group              | `d2b` (singleton)                  |

VM names are validated at eval time:

- Regex: `^[a-z][a-z0-9-]*$`.
- Reserved prefix: `sys-` (only the framework declares `sys-*` VMs).
- Reserved exact name: `launcher`.

Breaking any of these is a hard assertion in
`nixos-modules/assertions.nix`.

For the canonical glossary of internal identifiers (DAG node names,
bundle-relative artefact paths, broker op IDs) see
[`docs/reference/naming-conventions.md`](../reference/naming-conventions.md).

## Component split & sibling flakes

The **core framework** covers graphics, tpm, usbip, audio, network, the
auto-declared net VM, the per-VM store, the CLI, and the manifest contract.

Anything **identity- or workload-specific** lives in a sibling flake and is
composed per-VM:

- [`vicondoa/entrablau.nix`][entrablau] - Microsoft Entra ID
  joins (Himmelblau + TPM-bound machine credential).

Optional **desktop companion** pieces also live in sibling flakes:

- `vicondoa/d2b-toolkit` - shared Rust/Nix client DTOs, public-socket
  framing, redaction wrappers, Wayland color parsing, and Waybar helpers for
  desktop integrations.
- `vicondoa/d2b-wlterm` - Home Manager module and user-session launcher for
  persistent guest shells.
- `vicondoa/weezterm` - WeezTerm package/provider integration used by the
  terminal launcher when a d2b-aware terminal build is desired.

Consumer flakes combining these pieces keep one nixpkgs and toolkit revision
with `inputs.d2b.inputs.nixpkgs.follows = "nixpkgs"`,
`inputs.d2b-toolkit.inputs.nixpkgs.follows = "nixpkgs"`, and
`inputs.d2b-wlterm.inputs.d2b-toolkit.follows = "d2b-toolkit"`. WeezTerm
follows only `nixpkgs`; its flake does not expose a toolkit input. The exact
copy-paste boilerplate lives in
[`docs/how-to/configure-desktop-terminal-integration.md`](../how-to/configure-desktop-terminal-integration.md).

Composition is one-way: d2b core does not import identity, workload, or desktop
companion flakes. Identity/workload flakes stay d2b-agnostic; desktop
companions consume only d2b public CLI/socket contracts. Consumers compose
workload modules on a specific VM:

```nix
d2b.vms.work.config.imports = [
  inputs.entrablau.nixosModules.default
];
```

Before adding a sibling-shaped concern (e.g. a specific desktop environment or
dev-shell flavor) to core, consider its own flake. The core bar is: "every d2b
user plausibly wants this, and the framework cannot do the right thing without
it."

[entrablau]: https://github.com/vicondoa/entrablau.nix

## VM lifecycle (daemon-supervised)

`d2bd` solely supervises every per-VM lifecycle DAG. No framework-declared
per-VM systemd units: child processes (cloud-hypervisor, virtiofsd, swtpm,
vhost-user-sound, USBIP attach) are spawned by the broker via `SpawnRunner`, handed
back to `d2bd` over `SCM_RIGHTS` as pidfds, and reconciled against persisted
DAG state under `/var/lib/d2b/supervisor/state.json`.

Stop is provider-aware for local primary VMM runners. Normal
`d2b vm stop` asks Cloud Hypervisor guests to shut down via the CH
API and qemu-media guests via broker-mediated QMP before pidfd signal
cleanup. `--force` is an explicit operator override that skips only
that graceful guest wait and then uses the standard SIGTERM/SIGKILL
cleanup path. `d2b.daemon.lifecycle.gracefulShutdown.*` and
`d2b.vms.<vm>.lifecycle.gracefulShutdown.*` configure the bounded
wait; disabled VMs bypass the graceful phase without being marked
degraded.

The restart policy applies differently to the two daemon units (no
per-VM units are emitted):

- `d2bd.service` is `Type=notify` and may restart on switch/update.
  Systemd does not report it ready until the public socket is bound and
  the daemon has completed startup/adoption. `KillMode=process` ensures a
  daemon restart kills only the daemon main PID, not VM runner
  descendants; the restarted daemon re-adopts existing runners. The
  existing guarded `ExecStop` host-shutdown hook remains the all-VM
  teardown path and runs only when the system manager is stopping.
- `d2b-broker.service` is socket-activated. It reloads the
  current bundle resolver for each accepted request so a running broker
  does not dispatch stale runner intents after a switch, and it never
  holds in-flight session state across requests.

Drift detection moves from per-VM symlinks into the daemon state file.
`d2b vm list` flags a VM whose running closure differs from the latest declared
closure with `[pending restart]`; `d2b vm status <vm>` prints both store paths
and the exact remediation command (`d2b vm restart <vm>` for clean down+up,
`d2b vm switch <vm>` for per-VM closure rebuild plus live activation).

## Adding new per-VM behaviour

New per-VM work belongs **inside the daemon's DAG executor**
(`packages/d2bd-runtime/src/supervisor/`), with privileged effects routed through a
typed `d2b-broker` op declared in `packages/d2b-contracts/` and audited in
`/var/lib/d2b/audit/broker-<utc-date>.jsonl`. Do not introduce a
`systemd.services.*` declaration in `nixos-modules/` for per-VM work. Denylist coverage is owner-local or structural; run the focused
Nix-unit and daemon tests when changing this surface. See
[`docs/explanation/daemon-lifecycle.md`](../explanation/daemon-lifecycle.md)
for the DAG node taxonomy and
[`docs/reference/privileges.md`](../reference/privileges.md) for
the broker op catalogue.

Adding or reclassifying a spawned runner `ProcessRole` also needs matching
process-builder and role coverage: add or extend the typed Rust argv builder
and owner-local tests in the owning Provider crate in the same change.
