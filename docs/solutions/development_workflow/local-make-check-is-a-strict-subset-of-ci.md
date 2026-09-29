---
title: "make check passes on changes that CI rejects - the local gate is a strict subset of the CI matrix"
date: 2026-09-29
category: development_workflow
module: verification gates
problem_type: workflow_issue
component: development_workflow
symptoms:
  - "make check, check-census, and test-policy are all green, then CI reports failures on the same commit"
  - "Each CI failure is in a layer the local gates never touch: Nix parsing, test modules, clippy on test targets, Bazel meta checks"
  - "The failures look unrelated to the change, so the mental model is 'CI is flaky' rather than 'the local gate is incomplete'"
root_cause: config_error
resolution_type: configuration
severity: medium
tags: [ci, local-gates, clippy, nix, test-policy, verification, make-check]
---

# make check passes on changes that CI rejects - the local gate is a strict subset of the CI matrix

## Problem

A change set was verified green locally - `make check` (1023/1023), `make
check-census`, `make test-policy` (26/26), plus per-crate clippy and tests - and
still produced eight CI failures across two pushes. None of them were flakes and
none were in the code the local gates examine.

## Symptoms

- Local gates all pass; CI fails on the same SHA.
- Every failure is in a layer the local gate does not cover: Nix expression
  parsing, test-target clippy, generated-data freshness, or a Bazel meta check.
- The failures read as unrelated to the change, which invites the conclusion that
  CI is unreliable rather than that the local signal is partial.

## What Didn't Work

**Treating a green local run as evidence the change is shippable.** It is
evidence the change compiles and its tests pass. That is a strictly weaker
claim, and on this change set the difference was eight failures.

**Reading a CI failure as a flake on first sight.** Three of the eight were
worth chasing precisely because they looked like flakes. One of them was the
stale Bazel output-base assertion that gates on a row existing while it is
mid-delete, which is a genuine race - but it was found by running the check and
reading what it emitted, not by re-running it and getting green.

## Solution

Know what the local gate does not run, and treat CI as the first full
verification rather than a formality after it.

The local surface and the CI surface differ in at least these ways:

| local | CI |
|---|---|
| `cargo clippy` on **lib** targets | `cargo clippy --all-targets`, which includes **test** code |
| `cargo test` for touched crates | the full matrix, including `rust-main` |
| no Nix parse of changed `.nix` | `nix-instantiate --parse` over changed Nix files |
| `test-policy` on the committed inventory | the same, plus a regeneration-vs-edit distinction |
| no Bazel | `tier0` meta checks and per-check lane runs |

The eight failures this session, by which layer caught them:

- **`nix-unit`** - an appended test case landed without a separator between
  blocks, producing `syntax error, unexpected end of file`. The host's Lix parsed
  the file; the lane's pinned Nix could not express it at all.
- **`rust-broker`** - a doc line beginning with `//` was reworded as a numbered
  list, so rustdoc read it as a markdown list item. `useless_format` then fired
  on what was now a list.
- **`policy-tooling`** - the async-gate inventory recorded line numbers that a
  later code edit had shifted. This needed a regeneration, not a code change.
- **`rust-main`** - two byte-identical concatenated test functions.
- **`rust-broker`** - a test bound a Unix socket on a path longer than
  `sun_path` (108 bytes) because it built the path under the Bazel execroot.
- **`check` / `rust-main`** - `disallowed_methods` and a dead-code lib error that
  only surface under `--all-targets` with `-D warnings`.
- **`tier0`** - asserted on an output base whose row was being deleted, so it
  waited on the wrong thing.

## Why This Works

The local gates and CI are not two measurements of the same thing. The local
gate is a fast subset chosen for iteration speed; CI is the full matrix. A
green subset is not evidence about what the superset will say, and the failures
land in exactly the regions the subset excludes - which is also why they *look*
unrelated when they appear.

Two of these are not really CI-only problems, they are correctness problems the
local gate structurally cannot see:

- A Nix file the host's Lix accepts and the lane's pinned Nix cannot parse is a
  real portability break, and only a lane build reveals it.
- Test code that violates the repo's own `disallowed_methods` policy is a real
  policy violation. `cargo clippy` without `--all-targets` simply never
  compiles it.

## Prevention

**Before pushing, state which gates ran and which CI jobs are substitutes for
which local targets.** "All green locally" is not a claim about CI.

**When CI fails, find the layer that caught it before theorising about the
cause.** Asking "which job is this, and what does that job run that I did not"
converts each failure from a mystery into a known blind spot. Eight failures
stopped looking like eight unrelated problems once they were sorted by job.

**Expect that a passing filtered run is not a passing check.** One check in this
session passed 3 of 4 filtered runs - the passing runs complete in ~157s, while
the failing run is killed by a 180s stage bound and reports no stage. A green
subset of runs is not a green check.

**Do not re-run a failure hoping for green.** Capture the real failure text
first. In this session the fix for the one job that *was* a flake came from
reading the job log, not from rerunning it.

## Related Issues

- #614 - the NVRAM actor, which the local gate could not surface because the
  evidence only exists in a lane run
- #615 - a controller-session failure whose log line omits the stage, so the
  local gate had nothing to assert against
