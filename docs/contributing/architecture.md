# Architecture conventions

Naming rules the framework enforces at eval time, what belongs in this repo
versus a sibling flake, and how the daemon-supervised VM lifecycle is shaped.

The binding summary is in [`../../AGENTS.md`](../../AGENTS.md). The
daemon-only end-state itself is recorded in
[`../adr/0015-daemon-only-clean-break.md`](../adr/0015-daemon-only-clean-break.md),
which is the authority if this file drifts from it.

## Naming conventions

The framework declares **exactly three** root-visible units. There
is no `d2b@<vm>`-style per-VM unit; `d2bd` supervises every
per-VM DAG in-process and hands fds to spawned runners via the
broker's `SpawnRunner` / `OpenPidfd` ops.

| Resource                                | Pattern                                |
| --------------------------------------- | -------------------------------------- |
| Public daemon (supervisor)              | `d2bd.service`                     |
| Privileged broker socket                | `d2b-priv-broker.socket`           |
| Privileged broker service               | `d2b-priv-broker.service`          |
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

The **core framework** in this repo covers: graphics, tpm, usbip,
audio, network, the auto-declared net VM, the per-VM store, the
CLI, the manifest contract.

Anything **identity- or workload-specific** lives in a sibling
flake and is composed per-VM:

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

Consumer flakes that combine these pieces keep a single nixpkgs and toolkit
revision by using `inputs.d2b.inputs.nixpkgs.follows = "nixpkgs"`,
`inputs.d2b-toolkit.inputs.nixpkgs.follows = "nixpkgs"`, and
`inputs.d2b-wlterm.inputs.d2b-toolkit.follows = "d2b-toolkit"`. WeezTerm
follows only `nixpkgs`; its flake does not expose a toolkit input. The exact
copy-paste boilerplate lives in
[`docs/how-to/configure-desktop-terminal-integration.md`](../how-to/configure-desktop-terminal-integration.md).

The composition pattern is intentionally one-way: d2b core does not import
identity, workload, or desktop companion flakes. Identity/workload flakes can
stay d2b-agnostic; desktop companions consume only d2b's public CLI/socket
contracts. Consumers compose workload modules on a specific VM:

```nix
d2b.vms.work.config.imports = [
  inputs.entrablau.nixosModules.default
];
```

If you're tempted to add a new sibling-shaped concern (e.g. a
specific desktop environment, a particular dev-shell flavour) to
the core framework, consider whether it belongs in its own flake
instead. The bar for landing it in core is: "every d2b user
plausibly wants this, and the framework cannot do the right thing
without it."

[entrablau]: https://github.com/vicondoa/entrablau.nix

## VM lifecycle (daemon-supervised)

`d2bd` is the sole supervisor for every per-VM lifecycle DAG.
There are no framework-declared per-VM systemd units: child
processes (cloud-hypervisor, virtiofsd, swtpm, vhost-user-sound,
USBIP attach) are spawned by the broker via `SpawnRunner`, handed
back to `d2bd` over `SCM_RIGHTS` as pidfds, and reconciled
against the persisted DAG state under
`/var/lib/d2b/supervisor/state.json`.

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
- `d2b-priv-broker.service` is socket-activated. It reloads the
  current bundle resolver for each accepted request so a running broker
  does not dispatch stale runner intents after a switch, and it never
  holds in-flight session state across requests.

Drift detection moves from per-VM symlinks into the daemon's
state file. `d2b vm list` flags any VM where the running
closure differs from the latest declared closure with
`[pending restart]`; `d2b vm status <vm>` prints both store
paths and the exact remediation command (`d2b vm restart <vm>`
for a clean down+up, `d2b vm switch <vm>` for a per-VM closure
rebuild + live activation).

## Adding new per-VM behaviour

New per-VM work belongs **inside the daemon's DAG executor**
(`packages/d2bd/src/supervisor/`), with any privileged side
effects routed through a typed `d2b-priv-broker` op declared
in `packages/d2b-contracts/` and audited in
`/var/lib/d2b/audit/broker-<utc-date>.jsonl`. Do not introduce
a new `systemd.services.*` declaration in `nixos-modules/` for
per-VM work. The denylist coverage lives in
`packages/d2b-contract-tests/tests/policy_units.rs`; run the enabled
fixture-contract lane when changing this surface. See
[`docs/explanation/daemon-lifecycle.md`](../explanation/daemon-lifecycle.md)
for the DAG node taxonomy and
[`docs/reference/privileges.md`](../reference/privileges.md) for
the broker op catalogue.

Adding or reclassifying a spawned runner `ProcessRole` also requires
matching process-builder and runner-matrix coverage: add/extend the
typed Rust argv builder in `packages/d2b-host/src/*_argv.rs` and
the role coverage policy/contract tests under
`packages/d2b-contract-tests/tests/` in the same change.

