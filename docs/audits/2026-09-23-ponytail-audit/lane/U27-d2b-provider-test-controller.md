# U27 d2b-provider-test-controller

Lean already. Ship.

Checked: all five functions (`main`, `run`, `run_session`, `send_bootstrap`,
`controller_transport`, `should_reconnect`) have live in-process callers; every
import (Arc, VecDeque, Instant, OwnedFd, Duration, tracing macros, all
d2b-session/d2b-core-controller/d2b-session-unix symbols) has >=2 call sites,
nothing deletes as dead; persisted no unused deps (Cargo.toml deps all appear
in the source); no hand-rolled UUID/redaction/Deserialize; no stale BUILD
pair, no nix/ dir, no src/generated/ (out of per-crate scope anyway); both
in-file tests are behavioral (reconnect-vs-graceful disposition map,
one-bootstrap-then-terminal initial handshake via socketpair) and the lone
integration test is a fail-closed acceptance gate (no bootstrap fd -> non-zero
exit) - none tautological, none never-run. `should_reconnect` is the only
CloseReason-reconnect predicate in-tree (canonical CloseReason lives in
d2b-contracts-zone-session::v3::component_session, not duplicated here), so
there is no hand-rolled-duplicate class to surface DIY.

## Consistency notes

- Not a types-layer/contracts crate (U2-U11 range) - a provider fixture binary;
  no duplicate type definitions, naming-drift, or wire-shape-skew surface to
  reconcile. It correctly consumes `CONTROLLER_ASSIGNMENT_STREAM_ID` /
  `CONTROLLER_ASSIGNMENT_STREAM_CREDIT` from d2b-core-controller and
  `CloseReason` from d2b-contracts-zone-session rather than redefining them.
- `CONTROLLER_BOOTSTRAP_FD` (10) and `RUNTIME_FAILURE_EXIT` (78) are local
  consts matched to the host fixture's inherited-fd convention; no canonical
  home owns these.

## Reopened refusals

- none (no prior findings in ledger; nothing to reopen).

## Checked

Read `src/main.rs` verbatim (302 lines, both halves) and `tests/controller.rs`;
verified via `find`/`ls` that the crate ships only `src/main.rs` +
`tests/controller.rs` + `Cargo.toml` + `BUILD.bazel`; ran symbol-usage counts
for every imported name; grep-verified `should_reconnect` and the CloseReason
reconnect-decision have no duplicate repo-wide; confirmed `BUILD.bazel` (not a
stray `BUILD`) matches sibling provider crates; confirmed `src/generated/` is
absent; confirmed Cargo.toml deps map 1:1 to `use` statements. Ledger row for
U27 is "no prior findings" - honored, nothing refused, nothing new.
