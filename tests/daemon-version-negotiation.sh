#!/usr/bin/env bash
# Daemon version negotiation, deny_unknown_fields, and IfName validation.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
NL_LOG=${NL_LOG:-$ROOT/.agent-tmp/daemon-version-negotiation.log}
mkdir -p "$ROOT/.agent-tmp"

if [ -n "${NL_RUST_TOOLCHAIN_PATH:-}" ]; then
  PATH="$NL_RUST_TOOLCHAIN_PATH:$PATH"
  export PATH
fi

if [ -z "${NIXLING_DAEMON_VERSION_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    echo "daemon-version-negotiation: neither cargo nor nix is on PATH" >&2
    exit 1
  fi
  export NIXLING_DAEMON_VERSION_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

# shellcheck source=lib.sh
. "$HERE/lib.sh"

cd "$ROOT"

workspace_target_dir=$(nl_cargo_target_dir workspace)

scratch=$(nl_mktemp .daemon-version-negotiation.XXXXXX)
add_cleanup "rm -rf -- \"$scratch\""
mkdir -p "$scratch/run"
chmod 0755 "$scratch/run"
socket_path="$scratch/run/public.sock"
state_lock="$scratch/run/daemon.lock"
locks_dir="$scratch/run/locks"
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
  local expect_rc="$2"
  local expect_a="$3"
  local expect_b="${4:-}"
  shift 4 || true
  rm -f "$socket_path" "$state_lock"
  mkdir -p "$locks_dir"

  (
    export NIXLINGD_TEST_PEER_UID=60003
    export NIXLINGD_TEST_PEER_GID=60003
    export NIXLINGD_TEST_PEER_USERNAME=launcher-user
    export NIXLINGD_TEST_PEER_GROUPS=wheel
    cargo_nixlingd serve \
      --config "$config_json" \
      --test-listen-on "$socket_path" \
      --state-lock "$state_lock" \
      --locks-dir "$locks_dir" \
      --once \
      --allow-unprivileged-runtime-dir \
      --no-drop-privileges
  ) >"$scratch/${label}.server.log" 2>&1 &
  local server_pid=$!
  add_cleanup "kill $server_pid >/dev/null 2>&1 || true"
  wait_for_socket "$socket_path"

  local output rc
  set +e
  output=$(cargo_nixlingd test-client --socket "$socket_path" "$@" 2>&1)
  rc=$?
  set -e

  wait "$server_pid"
  assert_eq "$rc" "$expect_rc" "$label exit code"
  assert_contains "$output" "$expect_a" "$label primary match"
  if [ -n "$expect_b" ]; then
    assert_contains "$output" "$expect_b" "$label secondary match"
  fi
}

run_case version-mismatch 52 '"reason":"versionMismatch"' '"kind":"wire-version-mismatch"' \
  --frame-json '{"type":"hello","clientVersion":"<0.4.0","supportedFeatures":[]}'

run_case unknown-feature-flags 0 '"type":"helloOk"' '"type":"authStatusResponse"' \
  --frame-json '{"type":"hello","clientVersion":">=0.4.0, <0.5.0","supportedFeatures":["future-flag","future-flag-2"]}' \
  --frame-json '{"type":"authStatus"}'

run_case unknown-hello-field 51 '"type":"helloRejected"' '"kind":"wire-unknown-field"' \
  --frame-json '{"type":"hello","clientVersion":">=0.4.0, <0.5.0","supportedFeatures":[],"unexpected":true}'

run_case invalid-ifname 53 '"kind":"wire-ifname-invalid"' '"type":"error"' \
  --frame-json '{"type":"hello","clientVersion":">=0.4.0, <0.5.0","supportedFeatures":[]}' \
  --frame-json '{"type":"hostCheck","strict":false,"ifName":"abcdefghijklmnop"}'

ok "daemon version negotiation fails closed"
