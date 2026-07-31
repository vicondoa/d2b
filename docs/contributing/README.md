# Contributing docs

Detailed process documentation for people and agents changing
**`vicondoa/d2b` itself**. If you are *consuming* d2b in your own host
config, start at [`../../README.md`](../../README.md); if you are looking for
the rules rather than the detail, start at
[`../../AGENTS.md`](../../AGENTS.md).

[`../../AGENTS.md`](../../AGENTS.md) is the index and carries the binding
rules. These files carry the detail and the rationale behind them. Where the
two disagree, AGENTS.md wins; where either disagrees with committed, passing
code, the code wins.

| Doc | Covers |
| --- | --- |
| [workflow.md](./workflow.md) | Worktrees for parallel agents, the stacked-PR shape, integrator prep, edit/commit/validate, local host validation, screenshot hygiene, and the disk hygiene contract. |
| [panel-review.md](./panel-review.md) | The phase gate, fix-round scoping, the destructive-git rule for shared worktrees, the ten-role roster and each role's focus, and the swarm and unattended-run harness notes. |
| [changelog-and-commits.md](./changelog-and-commits.md) | Changelog fragments, the auto-release path, the changelog lifecycle at a version cut, the process-marker ban and its ratchet, and the full commit trailing-tag grammar. |
| [gates-and-lints.md](./gates-and-lints.md) | The heavy-lane semaphore, the spec-literal lint allowlist, and the D116 envelope negative-example marker. |
| [critical-subsystems.md](./critical-subsystems.md) | Full invariants for every subsystem in the AGENTS.md critical index, plus the cgroup slice naming and ownership-marker conventions. |
| [architecture.md](./architecture.md) | Eval-time naming rules, what belongs in a sibling flake, the daemon-supervised VM lifecycle, and how to add per-VM behaviour. |

These files are deliberately **not** listed as auto-loaded instruction files
for any agent harness. Loading them into every session is what made
AGENTS.md 122KB in the first place. Link to them; do not inline them.
