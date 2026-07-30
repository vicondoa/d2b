#!/usr/bin/env bash
# tests/test-proofs.sh - `make test-proofs`: clippy + test the standalone proof
# crates under proofs/ (separate Cargo workspaces, not members of packages/).
#
#   Discovery is automatic over proofs/*/Cargo.toml; an empty tree fails closed.
#
# These were previously only exercised by the hand-rolled pr-cargo-workspace CI
# job; they now live behind a make target so CI and local runs share one path.

set -euo pipefail
suite_started=$SECONDS

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
D2B_LOG=${D2B_LOG:-/dev/null}
export ROOT D2B_LOG

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"

cd "$ROOT"

toolchain_file="$ROOT/packages/rust-toolchain.toml"
pinned_channel=$(
  sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]\+\)".*/\1/p' "$toolchain_file" | head -1
)
[ -n "$pinned_channel" ] || { fail "could not read pinned Rust channel from $toolchain_file"; exit 1; }

d2b_activate_rust_toolchain_path || true

# Bootstrap the pinned toolchain through rustup/nix when cargo is absent (CI).
if [ -z "${D2B_PROOFS_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "cargo and nix both unavailable; cannot run proofs"
    exit 1
  fi
  log "  cargo not on PATH; re-entering via nix shell for pinned Rust $pinned_channel"
  export D2B_PROOFS_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" nixpkgs#rustup nixpkgs#stdenv.cc \
    --command bash -lc "
      set -euo pipefail
      rustup toolchain install '$pinned_channel' --profile minimal
      exec bash '$ROOT/tests/test-proofs.sh'
    "
fi

export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-$pinned_channel}"
# No RUSTC_WRAPPER override: the cargo configs route through
# .cargo/rustc-wrapper.sh, which degrades to plain rustc when sccache is
# absent, so clearing it here would only disable the compiler cache.

# Ensure the clippy component exists for the pinned toolchain. On CI runners
# that ship rustup pre-installed, cargo is already on PATH so the nix-shell
# bootstrap above is skipped; but the pinned toolchain then auto-installs as
# `minimal` (no clippy) on the first `cargo clippy`, which fails. Add clippy
# explicitly and idempotently whenever rustup drives the toolchain. (Locally,
# rustup is typically not on PATH - only the activated toolchain bin - and the
# toolchain already carries clippy, so this is a no-op.)
if command -v rustup >/dev/null 2>&1; then
  rustup toolchain install "$RUSTUP_TOOLCHAIN" --profile minimal >/dev/null 2>&1 || true
  rustup component add --toolchain "$RUSTUP_TOOLCHAIN" clippy
fi

# discovery is by directory, not by a hardcoded list: a hardcoded list paired
# with a silent "absent" skip lets a renamed or never-created proof crate pass
# the gate while executing nothing. Every proofs/*/Cargo.toml runs, and an
# empty proofs/ tree fails closed rather than reporting success over zero work.
rc=0
proofs=()
for manifest in "$ROOT"/proofs/*/Cargo.toml; do
  [ -f "$manifest" ] || continue
  lockfile="$(dirname "$manifest")/Cargo.lock"
  if [ ! -f "$lockfile" ]; then
    fail "proof manifest has no sibling Cargo.lock: ${manifest#"$ROOT/"}"
    exit 1
  fi
  proofs+=("$(basename "$(dirname "$manifest")")")
done

if [ "${#proofs[@]}" -eq 0 ]; then
  fail "no proof crates discovered under proofs/*/Cargo.toml"
  exit 1
fi

log "discovered ${#proofs[@]} proof crate(s): ${proofs[*]}"
for proof in "${proofs[@]}"; do
  manifest="$ROOT/proofs/$proof/Cargo.toml"
  log "--> proofs/$proof: clippy + test"
  if ! cargo clippy --locked --manifest-path "$manifest" --all-targets -- -D warnings \
    || ! cargo test --locked --manifest-path "$manifest"; then
    fail "proofs/$proof"
    rc=1
    continue
  fi

  # This proof's four scale fixtures are intentionally #[ignore]d so normal
  # developer cargo tests remain fast. The enforcing proof gate must execute
  # them explicitly: correctness, watches, conflict grouping, and owner fan-in
  # are the proof's principal oracles. Release mode keeps this roughly
  # five-minute lane inside the existing 15-minute CI job timeout.
  if [ "$proof" = "redb-resource-store-spike" ]; then
    log "    running required ignored full-scale suite (expected <=5 minutes)"
    if ! cargo test --release --locked --manifest-path "$manifest" \
      --test full_scale -- --ignored --test-threads=1 --nocapture; then
      fail "proofs/$proof full-scale suite"
      rc=1
      continue
    fi
  fi

  ok "proofs/$proof"
done

[ "$rc" -eq 0 ] || exit 1
log "test-proofs OK (duration: $((SECONDS - suite_started))s)"
