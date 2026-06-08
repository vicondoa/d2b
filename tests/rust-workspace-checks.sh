#!/usr/bin/env bash
# W0a Rust workspace checks. Called by tests/static.sh only when packages/ exists.
# If cargo is absent, re-enter through the repo-pinned nixpkgs toolchain.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

cd "$ROOT"

manifest="$ROOT/packages/Cargo.toml"
lock_file="$ROOT/packages/Cargo.lock"
deny_config="$ROOT/packages/deny.toml"
broker_manifest="$ROOT/packages/nixling-priv-broker/Cargo.toml"
broker_lock_file="$ROOT/packages/nixling-priv-broker/Cargo.lock"
broker_deny_config="$ROOT/packages/nixling-priv-broker/deny.toml"
for required in "$manifest" "$lock_file" "$deny_config" "$broker_manifest" "$broker_lock_file" "$broker_deny_config"; do
  if [ ! -f "$required" ]; then
    fail "missing Rust workspace input: $required"
    exit 1
  fi
done
toolchain_file="$ROOT/packages/rust-toolchain.toml"
pinned_channel=$(
  sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]\+\)".*/\1/p' "$toolchain_file" | head -1
)
if [ -z "$pinned_channel" ]; then
  fail "could not read pinned Rust channel from $toolchain_file"
  exit 1
fi
export pinned_channel

workspace_target_dir=$(nl_cargo_target_dir workspace)
broker_target_dir=$(nl_cargo_target_dir broker)

nl_activate_rust_toolchain_path || true
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-$pinned_channel}"

if [ -z "${NIXLING_RUST_GATE_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "neither cargo nor nix is on PATH; W0a rust gate cannot run"
    exit 1
  fi
  rust_gate_scratch=$(nl_mktemp .nixling-rust-gate.XXXXXX)
  add_cleanup "rm -rf -- \"$rust_gate_scratch\""
  log "  cargo not on PATH; re-entering via nix shell to acquire pinned Rust $pinned_channel toolchain"
  export NIXLING_RUST_GATE_IN_NIX_SHELL=1
  export NIXLING_RUST_GATE_BOOTSTRAP_RUSTUP=1
  export RUSTUP_HOME="$rust_gate_scratch/rustup"
  export CARGO_HOME="$rust_gate_scratch/cargo"
  nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#rustup nixpkgs#stdenv.cc nixpkgs#sccache \
    --command bash "$0" "$@"
  exit $?
fi

if [ -n "${NIXLING_RUST_GATE_IN_NIX_SHELL:-}" ]; then
  if [ -n "${NIXLING_RUST_GATE_BOOTSTRAP_RUSTUP:-}" ]; then
    log "--> rustup toolchain install $pinned_channel"
    rustup toolchain install "$pinned_channel" --profile minimal --component rustfmt --component clippy
    export PATH="$CARGO_HOME/bin:$PATH"
  else
    NIXLING_RUST_GATE_REAL_CARGO=$(command -v cargo)
    export NIXLING_RUST_GATE_REAL_CARGO
  fi
  rustc() {
    if [ -n "${NIXLING_RUST_GATE_BOOTSTRAP_RUSTUP:-}" ]; then
      command rustup run "$pinned_channel" rustc "$@"
    else
      command rustc "$@"
    fi
  }
  cargo() {
    local cargo_args=()
    if [ "$#" -ge 3 ] && [ "$1" = "--manifest-path" ]; then
      local manifest_arg=$2
      shift 2
      cargo_args=( "$1" --manifest-path "$manifest_arg" "${@:2}" )
    else
      cargo_args=( "$@" )
    fi
    if [ -n "${NIXLING_RUST_GATE_BOOTSTRAP_RUSTUP:-}" ]; then
      command rustup run "$pinned_channel" cargo "${cargo_args[@]}"
    else
      command "$NIXLING_RUST_GATE_REAL_CARGO" "${cargo_args[@]}"
    fi
  }
  export -f rustc
  export -f cargo
fi

assert_pinned_rust_toolchain() {
  local cargo_version rustc_version
  cargo_version=$(cargo --version)
  rustc_version=$(rustc --version)
  case "$cargo_version" in
    *"$pinned_channel"*) ;;
    *)
      fail "cargo version does not match packages/rust-toolchain.toml channel $pinned_channel: $cargo_version"
      exit 1
      ;;
  esac
  case "$rustc_version" in
    *"$pinned_channel"*) ;;
    *)
      fail "rustc version does not match packages/rust-toolchain.toml channel $pinned_channel: $rustc_version"
      exit 1
      ;;
  esac
  ok "Rust toolchain matches packages/rust-toolchain.toml ($pinned_channel)"
}

cleanup_cargo_special_files() {
  local label="$1" dir="$2"
  local removed=0
  while IFS= read -r path; do
    [ -n "$path" ] || continue
    rm -f -- "$path"
    removed=$((removed + 1))
  done < <(find "$dir" -type s -print 2>/dev/null || true)
  if [ "$removed" -gt 0 ]; then
    ok "$label removed $removed stale socket artifact(s) from $dir"
  fi
}

cleanup_package_test_scratch() {
  local label="$1" dir="$2"
  if [ -d "$dir" ]; then
    rm -rf -- "$dir"
    ok "$label removed package-local test scratch $dir"
  fi
}

log "--> rust toolchain version"
assert_pinned_rust_toolchain

log "--> cargo fmt --check"
cargo fmt --manifest-path "$manifest" --all --check
ok "cargo fmt --check"

log "--> cargo clippy --workspace --all-targets -- -D warnings"
CARGO_TARGET_DIR="$workspace_target_dir" cargo clippy --manifest-path "$manifest" --workspace --all-targets -- -D warnings
ok "cargo clippy"

log "--> cargo test --workspace"
CARGO_TARGET_DIR="$workspace_target_dir" cargo test --manifest-path "$manifest" --workspace
ok "cargo test"

# The privileged broker lives in its own sibling workspace, so the main
# workspace checks above do not see it. Validate its manifest/lock graph
# explicitly, then run its tests in both feature modes.
log "--> cargo metadata --format-version 1 (broker workspace)"
cargo metadata --format-version 1 --manifest-path "$broker_manifest" >/dev/null
ok "broker cargo metadata"

log "--> cargo check --workspace (broker workspace, default features = real wire dispatch)"
CARGO_TARGET_DIR="$broker_target_dir" cargo check --workspace --manifest-path "$broker_manifest"
ok "broker cargo check (default features = real wire dispatch)"

log "--> cargo check --workspace --features layer1-bootstrap (broker workspace, legacy probe-* harness)"
CARGO_TARGET_DIR="$broker_target_dir" cargo check --workspace --manifest-path "$broker_manifest" --features layer1-bootstrap
ok "broker cargo check --features layer1-bootstrap"

log "--> cargo test --workspace (broker workspace, default features = real wire dispatch)"
CARGO_TARGET_DIR="$broker_target_dir" cargo test --workspace --manifest-path "$broker_manifest"
ok "broker cargo test (default features = real wire dispatch)"

log "--> cargo test --workspace --features layer1-bootstrap (broker workspace, legacy probe-* harness)"
CARGO_TARGET_DIR="$broker_target_dir" cargo test --workspace --manifest-path "$broker_manifest" --features layer1-bootstrap
ok "broker cargo test --features layer1-bootstrap"

cleanup_cargo_special_files "workspace cargo test" "$workspace_target_dir"
cleanup_cargo_special_files "broker cargo test" "$broker_target_dir"
cleanup_package_test_scratch "workspace cargo test" "$ROOT/packages/nixlingd/target"

schema_out="$ROOT/packages/xtask/out"
schema_out_preexisting=0
if [ -e "$schema_out" ]; then
  schema_out_preexisting=1
fi
snapshot_schema_out() {
  if [ ! -d "$schema_out" ]; then
    return 0
  fi
  (
    cd "$schema_out"
    find . -type f -print0 \
      | LC_ALL=C sort -z \
      | xargs -0 -r sha256sum
  )
}

log "--> schema-drift placeholder (W1 will replace with real schemas)"
(cd "$ROOT/packages" && cargo xtask gen-schemas)
schema_snapshot_1=$(snapshot_schema_out)
(cd "$ROOT/packages" && cargo xtask gen-schemas)
schema_snapshot_2=$(snapshot_schema_out)
if [ "$schema_snapshot_1" != "$schema_snapshot_2" ]; then
  fail "schema-drift placeholder: cargo xtask gen-schemas output is not reproducible"
  diff -u \
    <(printf '%s\n' "$schema_snapshot_1") \
    <(printf '%s\n' "$schema_snapshot_2") >&2 || true
  exit 1
fi
if [ "$schema_out_preexisting" = "0" ]; then
  rm -rf -- "$schema_out"
fi
ok "schema-drift placeholder (W1 will replace with real schemas)"

cargo_deny_check() {
  local label="$1" manifest_path="$2" config_path="$3"
  if command -v cargo-deny >/dev/null 2>&1; then
    log "--> cargo deny check ($label)"
    cargo deny --manifest-path "$manifest_path" check --config "$config_path"
    ok "cargo deny check ($label)"
  elif command -v nix >/dev/null 2>&1; then
    log "--> cargo deny check ($label via nix shell)"
    nix shell --quiet --inputs-from "$ROOT" nixpkgs#cargo-deny --command \
      cargo deny --manifest-path "$manifest_path" check --config "$config_path"
    ok "cargo deny check ($label)"
  else
    fail "cargo deny check cannot run for $label: cargo-deny and nix are unavailable; ADR 0009 does not authorize a W0a waiver"
    exit 1
  fi
}

cargo_audit_check() {
  local label="$1" lock_path="$2"
  if command -v cargo-audit >/dev/null 2>&1; then
    log "--> cargo audit ($label)"
    cargo audit --file "$lock_path"
    ok "cargo audit ($label)"
  elif command -v nix >/dev/null 2>&1; then
    log "--> cargo audit ($label via nix shell)"
    nix shell --quiet --inputs-from "$ROOT" nixpkgs#cargo-audit --command \
      cargo audit --file "$lock_path"
    ok "cargo audit ($label)"
  else
    fail "cargo audit cannot run for $label: cargo-audit and nix are unavailable; ADR 0009 does not authorize a W0a waiver"
    exit 1
  fi
}

cargo_deny_check "main workspace" "$manifest" "$deny_config"
cargo_deny_check "broker workspace" "$broker_manifest" "$broker_deny_config"

cargo_audit_check "main workspace" "$lock_file"
cargo_audit_check "broker workspace" "$broker_lock_file"

log "--> tests/stub-no-socket.sh"
bash "$ROOT/tests/stub-no-socket.sh"
ok "stub-no-socket"
