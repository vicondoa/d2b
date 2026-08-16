### Fixed

- Corrected the static `d2b host check` refusal-contract fixtures and their
  documentation: the socket remediation names `d2bd.service`, the cgroup
  remediations name `d2b.slice`, and the fixture `docs_anchor` values resolve.
  These goldens are aspirational contract fixtures, not code-derived runtime
  output.
- The running `d2b host check` still emits finding-based output without a
  `docs_anchor` field and does not emit the code-specific socket or cgroup
  refusal envelopes. It therefore neither emits the stale `d2b-host.slice`
  remediation nor implements the corrected fixture contract.
- Corrected the AGENTS.md description of the envelope-lint negative-example
  exemption so it matches the lint: one exact, case-sensitive marker, honoured
  only in the single pinned documenting file and only when it appears once,
  with `policy_adr046_envelopes` named as the authority for the exact spelling.
- Corrected the contributor documentation for running heavy gates and folding
  changelog fragments: the `xtask` alias resolves only from `packages/`, so the
  documented invocation is now the root-safe
  `cargo run --manifest-path packages/Cargo.toml -p xtask -- ...`, with the
  `cd packages && cargo xtask ...` alternative noted for the `sccache` path.
