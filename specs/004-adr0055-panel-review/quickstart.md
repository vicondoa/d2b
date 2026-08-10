# Quickstart: Validate the Panel Review Cutover

## Prerequisites

- Work from the feature branch after all implementation commits.
- Ensure the worktree is clean.
- Use the repository-pinned Node.js and Rust toolchains.

## Selection and lifecycle behavior

Run:

```bash
make test-lint
```

Expected:

- mandatory and optional seat selection is deterministic;
- build changes select `build`, while citation-only prose does not;
- discovery, ledger, response, and verification artifacts are byte-stable;
- reviewer verdict schemas and adapter-produced normalized schemas remain
  distinct and exact;
- every actionable discovery finding, including MINOR and NIT, reaches the
  ledger and complete processing;
- `.complete` rejects in-place packet mutation and requires a new qualified
  round, while evidence-only rounds preserve the candidate snapshot;
- publication uses create-or-compare and only the two named atomic family
  directories, without generic filesystem safety machinery;
- legacy imports, stable `R` identifiers, monotonic roster widening, late issue
  rules, and metrics pass their behavior tests.

## Delivery validation

Run:

```bash
cargo test --manifest-path packages/Cargo.toml -p xtask
cargo clippy --manifest-path packages/Cargo.toml -p xtask --all-targets -- -D warnings
cargo fmt --manifest-path packages/Cargo.toml -p xtask -- --check
```

Expected:

- requests accept the selected roster;
- missing or extra records fail;
- complete legacy ten-seat records remain readable;
- accepted records remain candidate-bound and unanimous.

## Repository policy

Run:

```bash
make check-tier0
make test-changelog
make test-policy
```

Expected:

- prompt bindings and contributor documentation agree;
- shipped artifacts contain no process markers or non-ASCII dashes;
- the changelog fragment is valid.

## End-to-end review

Stage the finished candidate with the panel skill, run one discovery, create
the merged ledger and responses, then run scoped verification. The lifecycle
passes when every selected reviewer signs off with no blocking
recommendations. Every actionable finding enters the ledger. MINOR and NIT
become non-blocking only after their responses and verification statuses are
complete. Completed packet artifacts are process evidence, not an
authentication, privilege, secrecy, hostile-input, or same-UID security
boundary.
