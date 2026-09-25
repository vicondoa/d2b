# Rust CI workflows

The workflow file in full, ready to adapt, plus the variants. The posture
recorded in `docs/agents/rust.md` decides which sections apply — MSRV adds the
`msrv` job, `no_std` swaps the test job for a build, `unsafe` earns the Miri
job — and the base never ships unexamined.

## The base workflow

```yaml
name: Rust
on:
  push:
    branches: [main]
  pull_request:

concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true

permissions:
  contents: read

env:
  CARGO_TERM_COLOR: always

jobs:
  fmt:
    runs-on: ubuntu-latest
    timeout-minutes: 10
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt
      - run: cargo fmt --check

  clippy:
    runs-on: ubuntu-latest
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo clippy --all-targets --all-features

  test:
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --all-features
```

The clippy job carries no `-D warnings` on purpose: the level comes from the
repo `[lints]` block, and adding the flag would override a level the user
deliberately set. The flag belongs in this file only in the fallback form —
`cargo clippy --all-targets --all-features -- -D warnings` — for a repo with
no lint configuration at all.

## The MSRV job

An addition to the base, not a rewrite:

```yaml
  msrv:
    runs-on: ubuntu-latest
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@v4
      - name: Read MSRV from Cargo.toml
        id: msrv
        run: echo "version=$(cargo metadata --format-version 1 --no-deps | jq -r '.packages[0].rust_version')" >> "$GITHUB_OUTPUT"
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: ${{ steps.msrv.outputs.version }}
      - run: cargo build --all-features
```

This job builds and does not test: dev-dependencies frequently require a newer
compiler than the crate itself, and failing MSRV on a test-only dependency
teaches people to delete the job.

## The Miri job

Gated on the unsafe policy — included only when the repo actually contains
`unsafe`, and always on `nightly`:

```yaml
  miri:
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@nightly
        with:
          components: miri
      - run: cargo miri test
```

## The no_std variant

`--target thumbv7em-none-eabihf` plus `targets:` on the toolchain action:

```yaml
  build:
    runs-on: ubuntu-latest
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: thumbv7em-none-eabihf
      - run: cargo build --target thumbv7em-none-eabihf
```

`cargo test` cannot run on a bare-metal target, so this job builds only.

## A workspace note

Put `--workspace` on every command in a workspace, so a push that changes a
member reaches the check. `--all-features` is sometimes wrong: mutually
exclusive feature flags make the union fail to compile even though every
individual feature is fine. For that case the tool is `cargo hack` — run the
check per feature (or over the feature powerset) rather than with all of them
on. In a workspace, point the MSRV read at the root manifest; `.packages[0]`
does not guarantee the root package. A virtual workspace has no root package,
so `.packages[0].rust_version` can be `null`; there, read `rust-version` from
the workspace member that declares it.

## Three worked scenarios

**A single binary crate, no MSRV commitment.** The base workflow as written:
three jobs — `fmt`, `clippy`, `test`. No `msrv` job because nothing in the
repo commits to an old toolchain, and no Miri job because there is no
`unsafe`.

**A library with an MSRV and published docs.** The base, plus the `msrv` job,
plus a doc job that treats doc warnings as errors:

```yaml
  doc:
    runs-on: ubuntu-latest
    timeout-minutes: 20
    env:
      RUSTDOCFLAGS: -D warnings
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo doc --no-deps
```

**A workspace with existing CI.** The skill mostly reports the difference
between the existing workflow and what the recorded posture would produce, and
leaves things alone — the existing file wins unless the user asks for the
change.
