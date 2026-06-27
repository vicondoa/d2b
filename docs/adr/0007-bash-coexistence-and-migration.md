# 0007. Bash coexistence and migration

- Status: Accepted
- Date: 2026-05-25
- Wave: W0b
- Plan slice: "Migration modes: keep the current bash/systemd backend as legacy-systemd, add an opt-in daemon-experimental path, and flip to daemon-default only after parity and migration gates pass."
- Companion ADRs: ADR 0001, [ADR 0008](0008-supported-platforms-and-rejected-targets.md)

## Context

The v0.4.0 baseline ships a mature bash `d2b` CLI backed by
systemd and microvm.nix units. Its user-facing commands include
`list`, `status`, `up`, `down`, `switch`, `build`, `test`,
`rollback`, `boot`, `generations`, `gc`, `keys`, `audio`, `usb`,
`trust`, `rotate-known-host`, and `audit`.

That bash CLI already exposes JSON output for `d2b list`,
`d2b status <vm>`, `d2b keys list`, and `d2b audit`. The
portability plan introduces a Rust CLI shim in W2, then a Rust daemon
path through W4-W10. During that window, both implementations may exist
on the same host, so the plan needs explicit ownership rules that
prevent two writers from managing the same VM.

## Decision

1. D2b has three migration modes:
   `legacy-systemd`, the current bash plus systemd path;
   `daemon-experimental`, the Rust CLI and daemon path enabled per VM or
   component; and `daemon-default`, where the daemon owns eligible VMs
   by default.
2. Until the daemon-default flip, planned for W10, `d2b` as the
   Rust CLI binary is a thin shim. For each subcommand it either
   implements the operation natively against `d2bd` or execs the
   legacy bash CLI as a fallback. The allowlist of Rust-native
   subcommands is data-driven by an environment variable plus a manifest
   field.
3. The single-writer invariant is mandatory. Every VM declares
   `supervisor = "systemd" | "d2bd"` in its manifest. `d2bd`
   refuses to act on systemd-owned VMs. Systemd unit templates fail fast
   or no-op for daemon-owned VMs. A per-VM lock at
   `/run/d2b/locks/<vm>` guards coexistence paths.
4. The v0.4.0 bash `--json` outputs for `list`, `status <vm>`, and
   `keys list` are golden-test pinned in W2. Rust shim output must match
   byte-for-byte except for explicitly documented divergences logged via
   `CHANGELOG.md`.
5. D2b provides a compatibility window of at least one minor release
   where both backends coexist before the daemon-default flip. Bash-only
   paths are frozen one minor release before final removal.
6. W9 adds `d2b migrate`. The command converts existing per-VM state
   directories, generation symlinks, keys, current and booted symlinks,
   and SSH known-hosts entries. It also documents a rollback path back
   to `legacy-systemd`.

## Consequences

1. Positive: The single-writer invariant prevents split-brain between
   systemd and `d2bd` for one VM.
2. Positive: Existing v0.4.0 user-visible behavior remains the alpha
   compatibility target while Rust coverage expands subcommand by
   subcommand.
3. Positive: Operators get a tested off-ramp through `d2b migrate`
   and a documented rollback path before bash removal.
4. Negative: The shim must preserve both execution paths, JSON goldens,
   and fallback selection until the daemon-default release.
5. Neutral: Any intentional Rust-vs-bash output divergence must be
   called out in `CHANGELOG.md` rather than hidden in implementation
   details.

## Alternatives considered

- Replace the bash CLI as soon as the Rust binary exists: rejected
  because W2 does not yet provide lifecycle parity for the full v0.4.0
  command surface.
- Let systemd and `d2bd` race, with last-writer-wins behavior:
  rejected because VM lifecycle state, generation symlinks, keys, and
  sidecar ownership require one authoritative writer.
- Use a global migration switch only: rejected because alpha adoption
  needs per-VM and per-component opt-in while unsupported paths still
  fall back to bash.
- Treat JSON compatibility as best-effort: rejected because scripts and
  automation already consume the v0.4.0 `--json` outputs.

## References

- plan.md, "Baseline: d2b v0.4.0"
- plan.md, "Migration modes"
- plan.md, "W2 Rust workspace and API skeleton"
- plan.md, "W9 Packaging and onboarding"
- plan.md, "W10 Default switch and deprecation"
- [ADR 0006](0006-manifest-bundle-versioning.md)
- [ADR 0008](0008-supported-platforms-and-rejected-targets.md)
