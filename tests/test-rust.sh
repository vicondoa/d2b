#!/usr/bin/env bash
# tests/test-rust.sh — `make test-rust`: the comprehensive Rust gate.
#   fmt + clippy + `cargo test --workspace` (excluding the fixture-dependent
#   nixling-contract-tests), the contract crate against NL_FIXTURES, the
#   CLI-contract layer, no-bash-ast-walker, the privileged broker workspace
#   (3 feature passes, concurrent), schema-gen reproducibility, and cargo-deny.
# If cargo is absent, re-enter through the repo-pinned nixpkgs toolchain.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}

# shellcheck source=tests/lib.sh
. "$ROOT/tests/lib.sh"

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
# Separate target dirs for the broker's three concurrent feature passes so they
# don't lock-contend. They are DETERMINISTIC siblings of the broker target dir
# (not mktemp): sccache hashes the inherited CARGO_* environment, including
# CARGO_TARGET_DIR, so a random per-run target dir would change the cache key
# and defeat cross-run hits. Stable, distinct dirs keep the key stable (cache
# hits) while still avoiding lock contention. They are gitignored and reused
# across runs like the default broker/workspace target dirs.
broker_target_dir=$(nl_cargo_target_dir broker)
broker_layer1_target_dir="${broker_target_dir%/}-layer1"
broker_fakebackends_target_dir="${broker_target_dir%/}-fakebackends"

# Keep fixture-dependent contract crates out of generic workspace tests.
# Full NL_FIXTURES delivery to the sandbox/CI is a tracked W1 deliverable.
workspace_test_excludes=(--exclude nixling-contract-tests)

nl_activate_rust_toolchain_path || true
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-$pinned_channel}"

if [ -z "${NIXLING_RUST_GATE_IN_NIX_SHELL:-}" ] && ! command -v rustup >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "rustup not on PATH and nix is unavailable; rust gate cannot run pinned Rust $pinned_channel"
    exit 1
  fi
  rust_gate_scratch=$(nl_mktemp .nixling-rust-gate.XXXXXX)
  add_cleanup "rm -rf -- \"$rust_gate_scratch\""
  log "  rustup not on PATH; re-entering via nix shell to acquire pinned Rust $pinned_channel toolchain"
  export NIXLING_RUST_GATE_IN_NIX_SHELL=1
  export NIXLING_RUST_GATE_BOOTSTRAP_RUSTUP=1
  export RUSTUP_HOME="$rust_gate_scratch/rustup"
  export CARGO_HOME="$rust_gate_scratch/cargo"
  nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#rustup nixpkgs#stdenv.cc nixpkgs#sccache \
    --command bash "$0" "$@"
  exit $?
fi

if [ -z "${NIXLING_RUST_GATE_IN_NIX_SHELL:-}" ] && command -v rustup >/dev/null 2>&1; then
  export NIXLING_RUST_GATE_IN_NIX_SHELL=1
  export NIXLING_RUST_GATE_BOOTSTRAP_RUSTUP=1
  export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
  export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
  rustup toolchain install "$pinned_channel" --profile minimal --component rustfmt --component clippy
fi

if [ -z "${NIXLING_RUST_GATE_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "neither cargo nor nix is on PATH; rust gate cannot run"
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

# sccache: a per-crate compilation cache (keyed on source + flags), shared
# across the main + broker workspaces and all feature passes — so the broker's
# rebuilds of crates the main workspace already compiled (nixling-core/host/ipc)
# and its three separate-target-dir feature passes become cache hits. Used
# locally by default. In CI it is OFF unless NL_CI_SCCACHE=1 is set, because it
# only helps when a persistent backend survives across runs. CI opts in by
# pointing SCCACHE_DIR at a directory it restores/saves via actions/cache — we
# deliberately use sccache's LOCAL-DISK backend (NOT SCCACHE_GHA_ENABLED): the
# native GHA backend needs ACTIONS_RUNTIME_TOKEN exported into this process's
# environment, where the untrusted crate code this gate compiles and runs
# (build scripts, proc-macros, `cargo test`) could read and exfiltrate it.
# actions/cache performs its I/O in its own action process and never exposes
# that token to `run:` steps. The per-command `RUSTC_WRAPPER=""` overrides below
# (xtask gen-schemas) intentionally opt out regardless of this mode.
_ci_active=0
if [ -n "${CI:-}" ] || [ -n "${GITHUB_ACTIONS:-}" ]; then
  _ci_active=1
fi
if [ "${NL_NO_SCCACHE:-0}" = 1 ] || ! command -v sccache >/dev/null 2>&1; then
  export RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER=""
  log "sccache: disabled (forced off or unavailable)"
elif [ "$_ci_active" = 1 ] && [ "${NL_CI_SCCACHE:-0}" != 1 ]; then
  export RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER=""
  log "sccache: disabled (CI without NL_CI_SCCACHE opt-in)"
else
  _sccache_bin=$(command -v sccache)
  export RUSTC_WRAPPER="$_sccache_bin" CARGO_BUILD_RUSTC_WRAPPER="$_sccache_bin"
  if [ "$_ci_active" = 1 ]; then
    log "sccache: enabled ($_sccache_bin; CI opt-in, local backend at ${SCCACHE_DIR:-default})"
  else
    log "sccache: enabled ($_sccache_bin)"
  fi
fi

log "--> rust toolchain version"
assert_pinned_rust_toolchain

# The privileged broker is a SEPARATE workspace with three independent feature
# passes (default, layer1-bootstrap, fake-backends), each on its OWN target dir.
# They share nothing with the main workspace and nothing with each other, so the
# three are run CONCURRENTLY among themselves in the broker section below — but
# AFTER the main-workspace section, not overlapping it, so they don't contend
# with the main workspace's timing-sensitive tests. With sccache the shared
# crates are cache hits across all streams. Set NL_NO_PARALLEL_BROKER=1 to force
# serial. Each stream logs to its own file; failures surface at reap.
broker_stream_default() {
  cargo metadata --format-version 1 --manifest-path "$broker_manifest" >/dev/null
  CARGO_TARGET_DIR="$broker_target_dir" cargo check --workspace --manifest-path "$broker_manifest"
  rm -f -- "$broker_target_dir"/debug/deps/socket_activation-* 2>/dev/null || true
  CARGO_TARGET_DIR="$broker_target_dir" cargo test --workspace --manifest-path "$broker_manifest"
}
broker_stream_layer1() {
  CARGO_TARGET_DIR="$broker_layer1_target_dir" cargo check --workspace --manifest-path "$broker_manifest" --features layer1-bootstrap
  CARGO_TARGET_DIR="$broker_layer1_target_dir" cargo test --workspace --manifest-path "$broker_manifest" --features layer1-bootstrap
}
broker_stream_fakebackends() {
  CARGO_TARGET_DIR="$broker_fakebackends_target_dir" cargo test --workspace --manifest-path "$broker_manifest" --features fake-backends
}
broker_streams=(default layer1 fakebackends)
declare -A broker_pid broker_log
broker_parallel=0
[ "${NL_PARALLEL_BROKER:-0}" = 1 ] && broker_parallel=1

log "--> cargo fmt --check"
cargo fmt --manifest-path "$manifest" --all --check
ok "cargo fmt --check"

log "--> cargo clippy --workspace --all-targets -- -D warnings"
CARGO_TARGET_DIR="$workspace_target_dir" cargo clippy --manifest-path "$manifest" --workspace --all-targets -- -D warnings
ok "cargo clippy"

log "--> cargo test --workspace ${workspace_test_excludes[*]}"
CARGO_TARGET_DIR="$workspace_target_dir" cargo test --manifest-path "$manifest" --workspace "${workspace_test_excludes[@]}"
ok "cargo test"

# W3 fixture-contract layer: the nixling-contract-tests crate is EXCLUDED
# from the workspace test above because it reads the Nix-rendered bundle via
# $NL_FIXTURES. Build the fixture-smoke artifact and run the contract crate
# against it — this is what gates the fixture -> nixling-core DTO contract
# layer (e.g. the privileges Rust-vs-Nix matrix parity). Without this step
# the contract crate never runs in the gate.
if [ "${NL_SKIP_FIXTURE_BUILD:-0}" = 1 ]; then
  log "  SKIP: nixling-contract-tests (NL_SKIP_FIXTURE_BUILD=1; fixtures validated by flake-eval shards)"
elif command -v nix >/dev/null 2>&1; then
  log "--> cargo test -p nixling-contract-tests (NL_FIXTURES = fixture-smoke)"
  contract_system=$(nix eval --extra-experimental-features 'nix-command flakes' \
    --raw --impure --expr builtins.currentSystem 2>/dev/null || echo x86_64-linux)
  contract_fixtures=$(nix build --extra-experimental-features 'nix-command flakes' \
    --no-warn-dirty --no-link --print-out-paths "$ROOT#checks.${contract_system}.fixture-smoke")
  # Feature-rich fixture (graphics+video+audio+tpm+usbip+observability) for the
  # per-role minijail-validator contract tests — x86_64-linux only (graphics
  # platform gate). On other systems NL_FIXTURES_FULL stays unset and those
  # tests skip.
  contract_fixtures_full=""
  if [ "$contract_system" = "x86_64-linux" ]; then
    contract_fixtures_full=$(nix build --extra-experimental-features 'nix-command flakes' \
      --no-warn-dirty --no-link --print-out-paths "$ROOT#checks.${contract_system}.fixture-smoke-full")
  fi
  NL_FIXTURES="$contract_fixtures" NL_FIXTURES_FULL="$contract_fixtures_full" \
  CARGO_TARGET_DIR="$workspace_target_dir" \
    cargo test --manifest-path "$manifest" -p nixling-contract-tests
  ok "cargo test -p nixling-contract-tests (W3 fixture-contract layer)"

  # CLI-contract layer: spawn the real `nixling` binary against the rendered
  # fixture bundle (NL_FIXTURES) + a synthetic system-state and validate the
  # JSON envelopes strictly against the committed ListOutputV2/StatusOutputV2
  # DTOs (deny_unknown_fields). Successor of the cli-rust-native-* bash gates.
  #
  # A few CLI-contract cases (audit/host-check daemon-backed paths) spawn a
  # real, KVM-free `nixlingd serve --once --test-listen-on` and talk to it over
  # AF_UNIX + SO_PEERCRED. Build nixlingd and hand its path to the test via
  # NIXLING_TEST_NIXLINGD_BIN so those cases run instead of skipping. nixling
  # does NOT depend on nixlingd (the static-rust-dependency-direction gate
  # forbids it), so the path is delivered out-of-band rather than via a dep edge.
  log "--> cargo build -p nixlingd (CLI-contract daemon-spawn harness binary)"
  CARGO_TARGET_DIR="$workspace_target_dir" \
    cargo build --manifest-path "$manifest" -p nixlingd
  nixlingd_bin="$workspace_target_dir/debug/nixlingd"
  [ -x "$nixlingd_bin" ] || fail "nixlingd binary not found at $nixlingd_bin"
  log "--> cargo test -p nixling --tests (CLI-contract, NL_FIXTURES = fixture-smoke)"
  NL_FIXTURES="$contract_fixtures" \
  NIXLING_TEST_NIXLINGD_BIN="$nixlingd_bin" \
  CARGO_TARGET_DIR="$workspace_target_dir" \
    cargo test --manifest-path "$manifest" -p nixling --tests
  ok "cargo test -p nixling --tests (CLI-contract layer)"
else
  log "  SKIP: nixling-contract-tests (nix unavailable to build fixture-smoke)"
fi

# no-bash-exec AST layer (ADR 0017): the per-line `Command::new("bash")` scan
# is covered by nixling-contract-tests/tests/policy_source.rs, but the
# AST-level walk (which catches cross-line / obfuscated bash-exec sites the
# per-line regex cannot) lives in the standalone tests/tools/no-bash-ast-walker
# cargo tool. The retired tests/no-bash-exec-eval.sh ran it via `... all`; run
# it here so the AST coverage stays gated. Fails closed on any bash-literal
# Command::new site under packages/.
log "--> no-bash-ast-walker (ADR 0017 AST-level bash-exec scan)"
CARGO_TARGET_DIR="$workspace_target_dir" \
  cargo run --release --quiet \
    --manifest-path "$ROOT/tests/tools/no-bash-ast-walker/Cargo.toml" \
    -- "$ROOT/packages"
ok "no-bash-ast-walker (zero Command::new bash-literal sites)"

# Broker workspace: run the three feature passes (default, layer1-bootstrap,
# fake-backends) — each on its own target dir — serially by default because
# the broker's SIGCHLD reaper tests manipulate process-global signal/reap state.
# Set NL_PARALLEL_BROKER=1 only for local timing experiments. The fail-closed
# `fake-backends` stream runs the broker's hermetic
# integration tests (e.g. tests/pidfd_handoff_scm_rights.rs,
# #![cfg(feature = "fake-backends")], pinned in
# tests/golden/pinned/pidfd-handoff.txt) that neither the default nor the
# layer1-bootstrap pass enables — without it those fd-passing tests would not
# run in the gate at all (the retired tests/pidfd-handoff.sh used --all-features).
if [ "$broker_parallel" = 1 ]; then
  log "--> broker workspace: running default|layer1|fake-backends concurrently (separate target dirs)"
  broker_logdir=$(nl_mktemp ".nixling-broker-logs.XXXXXX")
  for _stream in "${broker_streams[@]}"; do
    broker_log[$_stream]="$broker_logdir/$_stream.log"
    ( "broker_stream_$_stream" ) >"${broker_log[$_stream]}" 2>&1 &
    broker_pid[$_stream]=$!
  done
  broker_failed=0
  for _stream in "${broker_streams[@]}"; do
    if wait "${broker_pid[$_stream]}"; then
      ok "broker cargo ($_stream feature pass)"
    else
      log "broker stream '$_stream' FAILED — captured output follows:"
      cat "${broker_log[$_stream]}" >&2 || true
      broker_failed=1
    fi
  done
  [ "$broker_failed" -eq 0 ] || { fail "broker workspace checks failed"; exit 1; }
else
  for _stream in "${broker_streams[@]}"; do
    log "--> broker cargo ($_stream feature pass, serial)"
    "broker_stream_$_stream"
    ok "broker cargo ($_stream feature pass)"
  done
fi

cleanup_cargo_special_files "workspace cargo test" "$workspace_target_dir"
cleanup_cargo_special_files "broker cargo test" "$broker_target_dir"
cleanup_cargo_special_files "broker layer1 cargo test" "$broker_layer1_target_dir"
cleanup_cargo_special_files "broker fake-backends cargo test" "$broker_fakebackends_target_dir"
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

log "--> schema generation reproducibility"
(cd "$ROOT/packages" && RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER="" cargo xtask gen-schemas)
schema_snapshot_1=$(snapshot_schema_out)
(cd "$ROOT/packages" && RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER="" cargo xtask gen-schemas)
schema_snapshot_2=$(snapshot_schema_out)
if [ "$schema_snapshot_1" != "$schema_snapshot_2" ]; then
  fail "schema generation reproducibility: cargo xtask gen-schemas output is not reproducible"
  diff -u \
    <(printf '%s\n' "$schema_snapshot_1") \
    <(printf '%s\n' "$schema_snapshot_2") >&2 || true
  exit 1
fi
if [ "$schema_out_preexisting" = "0" ]; then
  rm -rf -- "$schema_out"
fi
ok "schema generation reproducibility"

cargo_deny_check() {
  local label="$1" manifest_path="$2" config_path="$3"
  if command -v cargo-deny >/dev/null 2>&1; then
    log "--> cargo deny check ($label)"
    RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER="" \
      cargo deny --manifest-path "$manifest_path" check --config "$config_path"
    ok "cargo deny check ($label)"
  elif command -v nix >/dev/null 2>&1; then
    log "--> cargo deny check ($label via nix shell)"
    nix shell --quiet --inputs-from "$ROOT" nixpkgs#cargo-deny --command \
      env RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER="" \
        cargo deny --manifest-path "$manifest_path" check --config "$config_path"
    ok "cargo deny check ($label)"
  else
    fail "cargo deny check cannot run for $label: cargo-deny and nix are unavailable; ADR 0009 does not authorize a waiver"
    exit 1
  fi
}

cargo_audit_check() {
  local label="$1" lock_path="$2"
  if command -v cargo-audit >/dev/null 2>&1; then
    log "--> cargo audit ($label)"
    RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER="" cargo audit --file "$lock_path"
    ok "cargo audit ($label)"
  elif command -v nix >/dev/null 2>&1; then
    log "--> cargo audit ($label via nix shell)"
    nix shell --quiet --inputs-from "$ROOT" nixpkgs#cargo-audit --command \
      env RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER="" cargo audit --file "$lock_path"
    ok "cargo audit ($label)"
  else
    fail "cargo audit cannot run for $label: cargo-audit and nix are unavailable; ADR 0009 does not authorize a waiver"
    exit 1
  fi
}

cargo_deny_check "main workspace" "$manifest" "$deny_config"
cargo_deny_check "broker workspace" "$broker_manifest" "$broker_deny_config"

cargo_audit_check "main workspace" "$lock_file"
cargo_audit_check "broker workspace" "$broker_lock_file"

log "--> tests/tools/stub-no-socket.sh"
bash "$ROOT/tests/tools/stub-no-socket.sh"
ok "stub-no-socket"

# Fail-closed Rust test inventory: every pinned workspace + broker test must
# still exist (catches a silently-deleted test that would otherwise vanish from
# coverage). The pinned set is committed under tests/golden/pinned/.
log "--> tests/tools/assert-pinned-tests.sh"
bash "$ROOT/tests/tools/assert-pinned-tests.sh"
ok "assert-pinned-tests"
