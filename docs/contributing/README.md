# Contributing docs

Process detail for people and agents changing **`vicondoa/d2b` itself**. If
you consume d2b in your host config, start at [`../../README.md`](../../README.md);
for rules rather than detail, start at
[`../../AGENTS.md`](../../AGENTS.md).

[`../../AGENTS.md`](../../AGENTS.md) is the binding-rules index. These files
carry detail and rationale. Where they disagree, AGENTS.md wins; where either
disagrees with committed, passing code, code wins.

| Doc | Covers |
| --- | --- |
| [workflow.md](./workflow.md) | Worktrees, stacked PRs, integrator prep, edit/commit/validate, local host validation, screenshot hygiene, and disk hygiene. |
| [panel-review.md](./panel-review.md) | Phase gate, fix-round scope, shared-worktree destructive-git rule, ten-role focus, and swarm/unattended harness notes. |
| [changelog-and-commits.md](./changelog-and-commits.md) | Changelog fragments, auto-release, version-cut lifecycle, process-marker ratchet, and commit trailing-tag grammar. |
| [gates-and-lints.md](./gates-and-lints.md) | Heavy-lane semaphore, spec-literal allowlist, and D116 negative-example marker. |
| [critical-subsystems.md](./critical-subsystems.md) | Invariants for every AGENTS.md critical-index subsystem, plus cgroup naming and ownership markers. |
| [copilot-agents.md](./copilot-agents.md) | Copilot agents and skills, role agents, panel seats, autopilot/memory skills, model binding, wave identifiers, and spec-kit coexistence. |
| [architecture.md](./architecture.md) | Eval-time naming, sibling-flake boundaries, daemon-supervised VM lifecycle, and per-VM behavior. |

These files are deliberately **not** auto-loaded by agent harnesses. Loading
them into every session made AGENTS.md 122KB in the first place. Link to them;
do not inline them.
