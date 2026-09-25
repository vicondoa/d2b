---
name: setup-rust-ci
description: Write a GitHub Actions workflow for a Rust repo — format, clippy at the configured level, tests, and an MSRV job, derived from the posture the repo already recorded.
disable-model-invocation: true
---

# Setup Rust CI

CI is a promise the tooling keeps in public. The workflow this skill writes runs
on every push and pull request, and the first thing a contributor learns about
the repo is the tick or the cross. It runs only when the user invokes it — a
workflow executes on someone's behalf, and the model never decides that on its
own. The whole run is governed by one rule: **read the recorded posture first,
and make CI say exactly what the local tooling says.**

## What this writes, and the one rule

One workflow file, `.github/workflows/rust.yml`, plus a line in the contributing
notes the repo keeps, if it has them. The rule: **the CI job runs the same
commands the skills run locally, at the same level, or CI is lying.** A workflow
whose clippy invocation is stricter than the repo `[lints]` block produces
failures nobody can reproduce; one that is laxer produces a green tick that
means nothing. The full file, and the variants, are in `WORKFLOWS.md`.

## Read the posture first

`docs/agents/rust.md` — written by `/setup-rust-skills` — carries edition,
MSRV, async runtime, `no_std` posture, and unsafe policy. Every one of those
changes the workflow: MSRV decides whether there is a second toolchain job,
`no_std` decides whether `--target` matters, and the unsafe policy decides
whether a Miri job earns its slot. If the file is absent, say so and offer to
run `/setup-rust-skills` first — do not guess a posture and bake the guess
into CI.

## The three jobs that always earn their place

`fmt` (`cargo fmt --check`), `clippy` at the configured level, and `test`.
Each with its own job so a red tick names the failure without opening logs.
`build` is not a fourth default job — it earns its place in exactly two
spots: the `no_std` variant, where `cargo test` cannot run on a bare-metal
target so the job builds only, and the MSRV job, whose command is
`cargo build`. State plainly what does not belong in the default set: a
coverage gate, a benchmark run, and a cross-compilation matrix are all
things a repo adds when it needs them, not things a setup skill imposes.

## MSRV is a job, not a hope

If `rust-version` is set in `Cargo.toml`, CI must build on exactly that
toolchain, because nothing else will catch the day a contributor uses a newer
standard-library method. The version, not the ref, is what is pinned: the
`toolchain` input takes the MSRV read from `Cargo.toml`, never `stable`, as the
read step in `WORKFLOWS.md` does. The job builds and does not test —
`WORKFLOWS.md` says why.

## What makes a workflow safe to hand someone

- `permissions: contents: read` at the top level, widened only per job that
  needs it — a workflow that cannot write needs no audit for writes.
- Every action carries an explicit version ref — a floating ref lets the
  workflow change underneath the repo without the repo changing.
- A `concurrency` group keyed on the ref — a force-push cancels the
  superseded run instead of paying for two builds of the same code.
- `timeout-minutes` on each job — a hung job is an open-ended bill, and the
  timeout is what closes it.
- Caching that keys on the lockfile — the cache is valid exactly while the
  dependency set is unchanged, no wider.

Each has to survive a reviewer asking why it is there, because a workflow the
user cannot justify to their own reviewer will be deleted.

## Never overwrite an existing workflow

If `.github/workflows/` already holds a Rust workflow, read it and report the
difference as a diff for approval. A repo with CI has CI for reasons that
predate this skill.

## Verification

Before committing the workflow, prove every command in it actually passes
locally — with the `WORKFLOWS.md` caveat that `--all-features` is sometimes
wrong (mutually exclusive features), a repo in that case proves at the feature
set its CI actually runs:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features   # at the level the repo configured
cargo test --all-features
actionlint .github/workflows/rust.yml   # if available; report if not installed
```

Then, after pushing, confirm the first run:
`gh run list --workflow rust.yml --limit 1`. A workflow that has never run once
is not evidence of anything.
