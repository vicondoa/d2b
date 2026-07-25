### Fixed

- The `d2b host check` socket-permission diagnostic now directs operators to
  restart `d2bd.service` - which recreates `/run/d2b/public.sock` and
  re-asserts its mode, owner, and group on bind - instead of a `d2bd.socket`
  unit the framework does not declare, and its documentation link now resolves.
- Every `d2b host check`, `host prepare`, and `host destroy` error code now
  resolves to a stable anchor in `docs/reference/error-codes.md`, so the
  `docs_anchor` an operator follows from a diagnostic lands on a real entry
  rather than a dangling link.
- The `d2b host check` cgroup diagnostics now name the canonical `d2b.slice`
  in their remediation, matching the slice the broker actually creates and
  delegates, instead of a `d2b-host.slice` unit that does not exist.
- Corrected the AGENTS.md description of the envelope-lint negative-example
  exemption so it matches the lint: one exact, case-sensitive marker, honoured
  only in the single pinned documenting file and only when it appears once,
  with `policy_adr046_envelopes` named as the authority for the exact spelling.
- Corrected the contributor documentation for running heavy gates and folding
  changelog fragments: the `xtask` alias resolves only from `packages/`, so the
  documented invocation is now the root-safe
  `cargo run --manifest-path packages/Cargo.toml -p xtask -- ...`, with the
  `cd packages && cargo xtask ...` alternative noted for the `sccache` path.
