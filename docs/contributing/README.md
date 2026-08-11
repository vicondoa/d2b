# Contributing docs

Process detail for people and agents changing **`vicondoa/d2b` itself**. If
you consume d2b in your host config, start at [`../../README.md`](../../README.md);
for rules rather than detail, start at
[`../../AGENTS.md`](../../AGENTS.md).

[`../../AGENTS.md`](../../AGENTS.md) is the binding rules index. These files
carry detail and rationale. Where they disagree, AGENTS.md wins; where either
disagrees with committed, passing code, code wins.

| Doc | Covers |
| --- | --- |
| [workflow.md](./workflow.md) | Worktrees, stacked-PR shape, integrator prep, edit/commit/validate, local host validation, screenshot hygiene, and disk hygiene. |
| [panel-review.md](./panel-review.md) | Phase gate, Discover-Fix-Verify lifecycle, selected-roster focus, shared worktree destructive-git rule, and swarm and unattended-run harness notes. |
| [changelog-and-commits.md](./changelog-and-commits.md) | Changelog fragments, auto-release, version cut lifecycle, process-marker ratchet, and commit trailing-tag grammar. |
| [gates-and-lints.md](./gates-and-lints.md) | The heavy-lane semaphore, spec-literal allowlist, and D116 negative-example marker. |
| [critical-subsystems.md](./critical-subsystems.md) | Invariants for every AGENTS.md critical index subsystem, plus cgroup naming and ownership-marker conventions. |
| [copilot-agents.md](./copilot-agents.md) | Copilot agents and skills, role agents, panel seats, autopilot and memory skills, model-binding, wave identifiers, and spec-kit coexistence. |
| [architecture.md](./architecture.md) | Eval-time naming, sibling flake boundaries, daemon-supervised VM lifecycle, and per-VM behavior. |
| [gas-city.md](./gas-city.md) | Optional host-native Gas City contributor infrastructure: module deployment, credentials, ACP profiles, sidecars, lifecycle, publication, diagnostics, and live acceptance. It does not use the standalone d2b panel or wave-delivery path. |

These files are deliberately **not** auto-loaded by agent harnesses. Loading
them into every session made AGENTS.md 122KB in the first place. Link to them;
do not inline them.

The current panel process is the ADR 0055 Discover-Fix-Verify lifecycle:
deterministic selected-roster discovery, one shared ledger, batched
implementation responses and self-verification, and scoped verification.
`panel-review.md` and `copilot-agents.md` describe the contributor contract;
the versioned selection artifact is the roster authority shared by lifecycle
scripts and xtask delivery.
