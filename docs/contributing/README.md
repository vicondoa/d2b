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

These files are deliberately **not** auto-loaded by agent harnesses. Loading
them into every session made AGENTS.md 122KB in the first place. Link to them;
do not inline them.

## Migration from d2b Gas City exports

The repository-local Gas City contributor environment is retired. Contributor
orchestration now belongs to
[`vicondoa/d2b-gascity`](https://github.com/vicondoa/d2b-gascity), while NixOS
host distribution and installation belong to
[`vicondoa/gascity.nix`](https://github.com/vicondoa/gascity.nix).

Consumers must complete this order before adopting the d2b revision that
removes the exports:

1. Migrate the host configuration to `gascity.nix` and the portable city to
   `d2b-gascity`.
2. Run the standalone smoke checks and capture rollback evidence with the
   external owners.
3. Adopt the d2b export-removal revision. Rollback means pinning the prior d2b
   revision that still exposes these exports; d2b does not run that rollback
   locally.

The following retired identifiers are retained only as migration references:

- Package outputs: `packages.<system>.gascity`,
  `packages.<system>.gas-city-contributor`,
  `packages.<system>.gasCityContributor`, `packages.<system>.dolt`,
  `packages.<system>.beads`, and `packages.<system>.copilot`.
- Module and option namespace: `nixosModules.gasCityContributor` and
  `services.gasCityContributor`.
- Internal helper, shell, and check: `gasCityPackageSmokeFor`,
  `devShells.<system>.gas-city`, and
  `checks.<system>.gas-city-package-smoke`.

d2b does not migrate, delete, chmod, chown, or sweep existing
`/var/lib/gascity*` or `/run/gascity*` state. Host-state ownership and any
consumer rollback proof remain with the external owners.
