### Added

- The complete agent-skill surface is now vendored and committed so a
  fresh clone is fully configured when omp opens, with no install step:
  the `.agents/skills` and `.claude/skills` links into
  `third_party/agent-skills/` cover the Compound Engineering plugin's
  full `ce-*` set (v3.28.2, including the simplify and review addons),
  the caveman suite (v2.7.0), ponytail (v4.9.0), and the rewrite-rs Rust
  suite (v0.1.0-alpha.1) - 83 skills in total, in both link surfaces.
- The rewrite-rs import takes its `rust`, `workflow`, and `misc` skill
  groups and leaves the `porting` group out; the skip is recorded in that
  tree's `UPSTREAM.json`.
- `make update-agent-skills` refreshes the vendored trees from their
  upstream repositories and regenerates both link directories; each
  vendored tree carries an `UPSTREAM.json` provenance manifest
  (repository, commit, version, vendor date, per-file sha256).
- `tests/tools/tier0-first-pass.sh` derives its vendored-content
  exemption from the committed `.agents/skills` links instead of
  hard-coded version paths, so refreshes do not require gate edits.
- A committed `.compound-engineering/config.yaml` pins the Compound
  Engineering artifact root to `docs`, so pipeline plans land in
  `docs/plans/` and captured solutions under `docs/` inside the
  repository.
