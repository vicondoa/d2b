### Added

- The complete agent-skill surface is now vendored and committed so a
  fresh clone is fully configured when omp opens, with no install step:
  `.agents/skills` links into `third_party/agent-skills/` cover the
  Compound Engineering plugin's full `ce-*` set (v3.28.2, including the
  simplify and review addons), the caveman suite (v2.7.0), and ponytail
  (v4.9.0) - 62 skills in total.
- `make update-agent-skills` refreshes the vendored trees from their
  upstream repositories and regenerates the `.agents/skills` links;
  each vendored tree carries an `UPSTREAM.json` provenance manifest
  (repository, commit, version, vendor date, per-file sha256).
- `tests/tools/tier0-first-pass.sh` now exempts vendored skill content
  by path pattern instead of hard-coded version paths, so refreshes do
  not require gate edits.
- A committed `.compound-engineering/config.yaml` pins the Compound
  Engineering artifact root to `docs` (plans in `docs/plans/`, solutions
  in `docs/solutions/`).
