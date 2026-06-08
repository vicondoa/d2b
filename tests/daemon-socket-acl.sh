#!/usr/bin/env bash
# Daemon public-socket ACL fail-closed matrix.
#
# This test runs in a fake SO_PEERCRED mode because the Layer-1 environment
# is intentionally unprivileged. The daemon honors
# NIXLINGD_TEST_PEER_{UID,GID,USERNAME,GROUPS} so the shell gate can model
# launcher / wheel / daemon principals without setuid helpers. TMPDIR may be
# redirected away from /tmp in constrained CI; keep the socket path short so it
# still fits inside AF_UNIX SUN_LEN when the repo itself lives in a deep
# worktree path.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
NL_LOG=${NL_LOG:-$ROOT/.agent-tmp/daemon-socket-acl.log}
mkdir -p "$ROOT/.agent-tmp"

if [ -n "${NL_RUST_TOOLCHAIN_PATH:-}" ]; then
  PATH="$NL_RUST_TOOLCHAIN_PATH:$PATH"
  export PATH
fi

if [ -z "${NIXLING_DAEMON_SOCKET_ACL_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    echo "daemon-socket-acl: neither cargo nor nix is on PATH" >&2
    exit 1
  fi
  export NIXLING_DAEMON_SOCKET_ACL_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

# shellcheck source=lib.sh
. "$HERE/lib.sh"

cd "$ROOT"

workspace_target_dir=$(nl_cargo_target_dir workspace)

scratch=$(nl_mktemp .daemon-socket-acl.XXXXXX)
add_cleanup "rm -rf -- \"$scratch\""
runtime_root="$scratch/run"
mkdir -p "$runtime_root"
chmod 0755 "$runtime_root"
socket_root=${TMPDIR:-$scratch}
mkdir -p "$socket_root"
socket_path="$socket_root/public.sock"
state_lock="$runtime_root/daemon.lock"
locks_dir="$runtime_root/locks"
config_json="$scratch/config.json"

cat > "$config_json" <<EOF
{
  "publicSocketPath": "$socket_path",
  "brokerSocketPath": "$scratch/priv.sock",
  "stateLockPath": "$state_lock",
  "locksDir": "$locks_dir",
  "daemonUser": "root",
  "daemonGroup": "root",
  "publicSocketGroup": "$(id -gn)",
  "launcherUsers": ["launcher-user"],
  "adminUsers": ["admin-user"],
  "serverVersion": "0.4.0",
  "acceptedClientVersionRange": ">=0.4.0, <0.5.0"
}
EOF

daemon_bin=$(nl_cargo_bin_path workspace nixlingd)
if [ ! -x "$daemon_bin" ]; then
  CARGO_TARGET_DIR="$workspace_target_dir" cargo build --manifest-path "$ROOT/packages/Cargo.toml" --quiet -p nixlingd
fi

cargo_nixlingd() {
  "$daemon_bin" "$@"
}

wait_for_socket() {
  local path="$1"
  local attempts=0
  while [ "$attempts" -lt 300 ]; do
    [ -S "$path" ] && return 0
    attempts=$((attempts + 1))
    sleep 0.1
  done
  fail "timed out waiting for socket: $path"
}

run_case() {
  local label="$1"
  local peer_uid="$2"
  local peer_username="$3"
  local peer_groups="$4"
  local expect_rc="$5"
  local expect_a="$6"
  local expect_b="${7:-}"
  local mode="${8:-reject}"
  local log_file="$scratch/${label}.server.log"
  rm -f "$socket_path" "$state_lock"
  mkdir -p "$locks_dir"

  (
    export NIXLINGD_TEST_PEER_UID="$peer_uid"
    export NIXLINGD_TEST_PEER_GID="$peer_uid"
    export NIXLINGD_TEST_PEER_USERNAME="$peer_username"
    export NIXLINGD_TEST_PEER_GROUPS="$peer_groups"
    cargo_nixlingd serve \
      --config "$config_json" \
      --test-listen-on "$socket_path" \
      --state-lock "$state_lock" \
      --locks-dir "$locks_dir" \
      --once \
      --allow-unprivileged-runtime-dir \
      --no-drop-privileges
  ) >"$log_file" 2>&1 &
  local server_pid=$!
  add_cleanup "kill $server_pid >/dev/null 2>&1 || true"
  if ! wait_for_socket "$socket_path"; then
    cat "$log_file" >&2 || true
    return 1
  fi

  local output rc
  set +e
  if [ "$mode" = success ]; then
    output=$(cargo_nixlingd test-client \
      --socket "$socket_path" \
      --frame-json '{"type":"hello","clientVersion":">=0.4.0, <0.5.0","supportedFeatures":[]}' \
      --frame-json '{"type":"authStatus"}' 2>&1)
  else
    output=$(cargo_nixlingd test-client \
      --socket "$socket_path" \
      --frame-json '{"type":"hello","clientVersion":">=0.4.0, <0.5.0","supportedFeatures":[]}' 2>&1)
  fi
  rc=$?
  set -e

  wait "$server_pid"
  assert_eq "$rc" "$expect_rc" "$label exit code"
  assert_contains "$output" "$expect_a" "$label primary match"
  if [ -n "$expect_b" ]; then
    assert_contains "$output" "$expect_b" "$label secondary match"
  fi
}

run_case non-launcher-uid 60001 random-user users 31 '"kind":"authz-not-a-launcher"' '"type":"helloRejected"'
run_case wheel-but-not-launcher 60002 wheel-user wheel 31 '"kind":"authz-not-a-launcher"' '"type":"helloRejected"'
run_case configured-launcher 60003 launcher-user wheel 0 '"type":"helloOk"' '"role":"launcher"' success
run_case daemon-self-client 0 daemon-user root 31 '"kind":"authz-not-a-launcher"' '"type":"helloRejected"'

ok "daemon public-socket ACL matrix fails closed"
