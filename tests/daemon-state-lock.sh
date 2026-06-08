#!/usr/bin/env bash
# Daemon OFD state-lock fail-closed behavior.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
NL_LOG=${NL_LOG:-$ROOT/.agent-tmp/daemon-state-lock.log}
mkdir -p "$ROOT/.agent-tmp"

if [ -n "${NL_RUST_TOOLCHAIN_PATH:-}" ]; then
  PATH="$NL_RUST_TOOLCHAIN_PATH:$PATH"
  export PATH
fi

if [ -z "${NIXLING_DAEMON_STATE_LOCK_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    echo "daemon-state-lock: neither cargo nor nix is on PATH" >&2
    exit 1
  fi
  export NIXLING_DAEMON_STATE_LOCK_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

# shellcheck source=lib.sh
. "$HERE/lib.sh"

cd "$ROOT"

workspace_target_dir=$(nl_cargo_target_dir workspace)

scratch=$(nl_mktemp .daemon-state-lock.XXXXXX)
add_cleanup "rm -rf -- \"$scratch\""
mkdir -p "$scratch/run"
chmod 0755 "$scratch/run"
config_json="$scratch/config.json"
lock_path="$scratch/run/daemon.lock"

cat > "$config_json" <<EOF
{
  "publicSocketPath": "$scratch/public.sock",
  "brokerSocketPath": "$scratch/priv.sock",
  "stateLockPath": "$lock_path",
  "locksDir": "$scratch/locks",
  "daemonUser": "root",
  "daemonGroup": "root",
  "publicSocketGroup": "$(id -gn)",
  "launcherUsers": [],
  "adminUsers": [],
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

cargo_nixlingd lock-only \
  --config "$config_json" \
  --state-lock "$lock_path" \
  --allow-unprivileged-runtime-dir \
  --hold-seconds 20 >"$scratch/first.log" 2>&1 &
first_pid=$!
add_cleanup "kill $first_pid >/dev/null 2>&1 || true"

for _ in $(seq 1 300); do
  [ -f "$lock_path" ] && break
  sleep 0.1
done
assert_file_exists "$lock_path"
assert_eq "$(stat -c '%a' "$lock_path")" "640" "state-lock mode"
assert_eq "$(stat -c '%u' "$lock_path")" "$(id -u)" "state-lock uid"
assert_eq "$(stat -c '%g' "$lock_path")" "$(id -g)" "state-lock gid"

set +e
second_output=$(cargo_nixlingd lock-only \
  --config "$config_json" \
  --state-lock "$lock_path" \
  --allow-unprivileged-runtime-dir \
  --hold-seconds 1 2>&1)
second_rc=$?
set -e
assert_eq "$second_rc" "41" "second daemon exits AlreadyRunning"
assert_contains "$second_output" 'internal-already-running' 'typed already-running error'

mkdir -p "$scratch/real-parent"
chmod 0755 "$scratch/real-parent"
ln -s "$scratch/real-parent" "$scratch/symlink-parent"
set +e
symlink_output=$(cargo_nixlingd lock-only \
  --config "$config_json" \
  --state-lock "$scratch/symlink-parent/daemon.lock" \
  --allow-unprivileged-runtime-dir \
  --hold-seconds 1 2>&1)
symlink_rc=$?
set -e
assert_eq "$symlink_rc" "42" "symlink parent fails closed"
assert_contains "$symlink_output" 'internal-lock-parent-invalid' 'typed lock-parent error'
assert_contains "$symlink_output" 'must not be a symlink' 'symlink rejection message'

kill "$first_pid" >/dev/null 2>&1 || true
wait "$first_pid" >/dev/null 2>&1 || true

ok "daemon state-lock OFD gate fails closed"
