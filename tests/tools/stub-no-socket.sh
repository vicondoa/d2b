#!/usr/bin/env bash
# Assert Rust CLI/daemon stubs exit cleanly without leaving runtime state.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}

# shellcheck source=tests/lib.sh
. "$ROOT/tests/lib.sh"

d2b_activate_rust_toolchain_path || true

manifest="$ROOT/packages/Cargo.toml"
workspace_target_dir=$(d2b_cargo_target_dir workspace)
if [ ! -f "$manifest" ]; then
  fail "missing Rust workspace manifest: $manifest"
  exit 1
fi

if [ -z "${D2B_STUB_NO_SOCKET_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "stub-no-socket: neither cargo nor nix is on PATH"
    exit 1
  fi
  export D2B_STUB_NO_SOCKET_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt nixpkgs#clippy nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

scratch=$(d2b_mktemp .d2b-stub-smoke.XXXXXX)
add_cleanup "rm -rf -- \"$scratch\""
test_home="$scratch/home"
test_tmp="$scratch/tmp"
test_runtime="$scratch/xdg-runtime"
install -d -m 0700 "$test_home" "$test_tmp" "$test_runtime"
export HOME="$test_home"
export TMPDIR="$test_tmp"
export XDG_RUNTIME_DIR="$test_runtime"

snapshot_tree() {
  local dir="$1"
  if [ -z "$dir" ] || [ ! -e "$dir" ]; then
    return 0
  fi
  if [ ! -d "$dir" ]; then
    printf '%s\n' "$dir"
    return 0
  fi
  if command -v sudo >/dev/null 2>&1 && sudo -n -A true >/dev/null 2>&1; then
    sudo -n -A find "$dir" -mindepth 1 -print 2>/dev/null | LC_ALL=C sort
  else
    find "$dir" -mindepth 1 -print 2>/dev/null | LC_ALL=C sort || true
  fi
}

snapshot_listdir() {
  local dir="$1"
  if [ -z "$dir" ] || [ ! -e "$dir" ]; then
    return 0
  fi
  if [ ! -d "$dir" ]; then
    printf '%s\n' "$dir"
    return 0
  fi
  if command -v sudo >/dev/null 2>&1 && sudo -n -A true >/dev/null 2>&1; then
    sudo -n -A find "$dir" -mindepth 1 -maxdepth 1 -print 2>/dev/null | LC_ALL=C sort
  else
    find "$dir" -mindepth 1 -maxdepth 1 -print 2>/dev/null | LC_ALL=C sort || true
  fi
}

assert_no_new_paths() {
  local label="$1"
  local before="$2"
  local after="$3"
  local added

  added=$(comm -13 <(printf '%s\n' "$before") <(printf '%s\n' "$after"))
  if [ -n "$added" ]; then
    fail "$label gained unexpected runtime state:" || true
    printf '%s\n' "$added" | sed 's/^/  - /' >&2
    exit 1
  fi
}

assert_path_not_modified() {
  local label="$1"
  local path="$2"
  local list_before="$3"
  local list_after=""

  list_after=$(snapshot_listdir "$path")

  if [ "$list_after" != "$list_before" ]; then
    fail "$label directory entries changed:" || true
    diff -u <(printf '%s\n' "$list_before") <(printf '%s\n' "$list_after") >&2 || true
    exit 1
  fi
}

assert_no_runtime_state() {
  local bin="$1"
  local run_list_before="$2"
  local var_lib_list_before="$3"
  local xdg_before="$4"
  local tmp_before="$5"
  local unexpected_home=()
  local xdg_after=""
  local tmp_after=""

  assert_path_not_modified "$bin /run/d2b" /run/d2b "$run_list_before"
  assert_path_not_modified "$bin /var/lib/d2b" /var/lib/d2b "$var_lib_list_before"

  if [ -n "${XDG_RUNTIME_DIR:-}" ]; then
    xdg_after=$(snapshot_tree "$XDG_RUNTIME_DIR")
    assert_no_new_paths "$bin XDG_RUNTIME_DIR ($XDG_RUNTIME_DIR)" "$xdg_before" "$xdg_after"
  fi

  tmp_after=$(snapshot_tree "$TMPDIR")
  assert_no_new_paths "$bin TMPDIR ($TMPDIR)" "$tmp_before" "$tmp_after"

  while IFS= read -r path; do
    rel=${path#"$test_home"/}
    case "$rel" in
      .cargo|.cargo/*|.rustup|.rustup/*)
        ;;
      *)
        unexpected_home+=( "$rel" )
        ;;
    esac
  done < <(find "$test_home" -mindepth 1 -print | LC_ALL=C sort)
  if [ "${#unexpected_home[@]}" -gt 0 ]; then
    fail "$bin created unexpected HOME state:" || true
    printf '  - %s\n' "${unexpected_home[@]}" >&2
    exit 1
  fi
}

run_stub() {
  local bin="$1"
  local expected="$2"
  local output rc
  local run_list_before=""
  local var_lib_list_before=""
  local xdg_before="" tmp_before=""

  log "--> cargo run --bin $bin"
  run_list_before=$(snapshot_listdir /run/d2b)
  var_lib_list_before=$(snapshot_listdir /var/lib/d2b)
  if [ -n "${XDG_RUNTIME_DIR:-}" ]; then
    xdg_before=$(snapshot_tree "$XDG_RUNTIME_DIR")
  fi
  tmp_before=$(snapshot_tree "$TMPDIR")

  set +e
  output=$(
    cd "$ROOT/packages" && \
      RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER="" \
      CARGO_TARGET_DIR="$workspace_target_dir" \
      cargo run --manifest-path "$manifest" --quiet --bin "$bin" 2>&1
  )
  rc=$?
  set -e

  if [ "$rc" -ne 0 ]; then
    fail "$bin exited $rc" || true
    printf '%s\n' "$output" >&2
    exit 1
  fi
  assert_contains "$output" "$expected" "$bin stdout"
  assert_no_runtime_state "$bin" "$run_list_before" "$var_lib_list_before" \
    "$xdg_before" "$tmp_before"
}

run_stub d2b "d2b 0.0.0-bootstrap (bootstrap stub)"
run_stub d2bd "d2bd 0.0.0-bootstrap (bootstrap stub)"

ok "Rust stubs left no socket/file runtime state"
