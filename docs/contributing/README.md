# Contributing docs

Process detail for people and agents changing **`vicondoa/d2b` itself**. If
you consume d2b in your host config, start at [`../../README.md`](../../README.md);
for product direction, start at [`../../STRATEGY.md`](../../STRATEGY.md); for
operational rules, start at [`../../AGENTS.md`](../../AGENTS.md).

[`../../AGENTS.md`](../../AGENTS.md) is the single binding operational
authority. These files carry focused detail and rationale; `STRATEGY.md` is
product direction only. Where the docs disagree, AGENTS.md wins; where either
disagrees with committed, passing code, code wins.

| Doc | Covers |
| --- | --- |
| [workflow.md](./workflow.md) | Isolated worktrees, task routing, reviewed-head PR lifecycle, landing, edit/commit/validate, local host validation, screenshot hygiene, and disk hygiene. |
| [changelog-and-commits.md](./changelog-and-commits.md) | Changelog fragments, auto-release, version cut lifecycle, release hygiene, and commit grammar. |
| [gates-and-lints.md](./gates-and-lints.md) | The heavy-lane semaphore and contributor validation lanes. |
| [critical-subsystems.md](./critical-subsystems.md) | Invariants for every AGENTS.md critical index subsystem, plus cgroup naming and ownership-marker conventions. |
| [architecture.md](./architecture.md) | Eval-time naming, sibling flake boundaries, daemon-supervised VM lifecycle, and per-VM behavior. |
| [gas-city.md](./gas-city.md) | Optional host-native Gas City contributor infrastructure: module deployment, credentials, ACP profiles, sidecars, lifecycle, publication, diagnostics, and live acceptance. |

These files are deliberately **not** auto-loaded by agent harnesses. Loading
them into every session made AGENTS.md 122KB in the first place. Link to them;
do not inline them.
