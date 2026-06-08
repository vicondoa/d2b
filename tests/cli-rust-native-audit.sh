#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=cli-rust-native-common.sh
. "$HERE/cli-rust-native-common.sh"

if [ -z "${NIXLING_CLI_RUST_NATIVE_AUDIT_IN_NIX_SHELL:-}" ] && ! command -v python3 >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "cli-rust-native-audit: neither python3 nor nix is on PATH"
    exit 1
  fi
  export NIXLING_CLI_RUST_NATIVE_AUDIT_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" nixpkgs#python3 --command bash "$0" "$@"
fi

log "==> tests/cli-rust-native-audit.sh"
scratch=$(nl_mktemp .cli-rust-native-audit.XXXXXX)
cleanup_cli_rust_native_audit() {
  if [ -n "${mock_pid:-}" ] && kill -0 "$mock_pid" 2>/dev/null; then
    kill "$mock_pid" >/dev/null 2>&1 || true
    wait "$mock_pid" || true
  fi
  if [ -n "${daemon_pid:-}" ] && kill -0 "$daemon_pid" 2>/dev/null; then
    kill "$daemon_pid" >/dev/null 2>&1 || true
    wait "$daemon_pid" || true
  fi
}
add_cleanup cleanup_cli_rust_native_audit

wait_for_socket() {
  local path="$1"
  local attempts=0
  while [ "$attempts" -lt 200 ]; do
    [ -S "$path" ] && return 0
    attempts=$((attempts + 1))
    sleep 0.05
  done
  fail "timed out waiting for socket: $path"
}

cli=$(nl_cli_native_bin)
legacy_poison="$scratch/legacy-poison.sh"
cat > "$legacy_poison" <<'EOF2'
#!/usr/bin/env bash
echo "FAIL: rust CLI exec'd the legacy bash poison-pill with args: $*" >&2
exit 99
EOF2
chmod +x "$legacy_poison"

set +e
NIXLING_LEGACY_CLI="$legacy_poison" \
NIXLING_LEGACY_CLI_PATH="$legacy_poison" \
NIXLING_LEGACY_BASH_OPT_IN=1 \
NIXLING_PUBLIC_SOCKET="$scratch/missing.sock" \
NIXLING_AUDIT_TESTMODE_KVM_MODE=660 \
  "$cli" audit --human > "$scratch/audit.human" 2> "$scratch/audit.human.stderr"
rc_human=$?
NIXLING_LEGACY_CLI="$legacy_poison" \
NIXLING_LEGACY_CLI_PATH="$legacy_poison" \
NIXLING_LEGACY_BASH_OPT_IN=1 \
NIXLING_PUBLIC_SOCKET="$scratch/missing.sock" \
NIXLING_AUDIT_TESTMODE_KVM_MODE=660 \
  "$cli" audit --json > "$scratch/audit.json" 2> "$scratch/audit.json.stderr"
rc_json=$?
set -e

if [ "$rc_human" -eq 1 ] \
  && [ "$rc_json" -eq 1 ] \
  && grep -Fq 'daemon-down' "$scratch/audit.human.stderr" \
  && jq -e '.code == "daemon-down" and .exitCode == 1' "$scratch/audit.json" >/dev/null 2>&1; then
  ok "audit reports typed daemon-down when nixlingd is unreachable"
else
  fail "audit daemon-down handling regressed"
  echo "--- audit --human stdout ---" >&2
  cat "$scratch/audit.human" >&2
  echo "--- audit --human stderr ---" >&2
  cat "$scratch/audit.human.stderr" >&2
  echo "--- audit --json stdout ---" >&2
  cat "$scratch/audit.json" >&2
  echo "--- audit --json stderr ---" >&2
  cat "$scratch/audit.json.stderr" >&2
  exit 1
fi

if [ "$rc_human" -ne 99 ] && [ "$rc_json" -ne 99 ]; then
  ok "audit does not fall back to legacy bash when nixlingd is unreachable"
else
  fail "audit reached the legacy bash poison-pill"
  exit 1
fi

cat > "$scratch/mock-daemon.py" <<'PY'
import json
import os
import socket
import struct
import sys

path = sys.argv[1]
mode = sys.argv[2]
try:
    os.unlink(path)
except FileNotFoundError:
    pass

server = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
server.bind(path)
server.listen(1)
conn, _ = server.accept()


def recv_frame():
    data = conn.recv(1048580)
    if len(data) < 4:
        raise SystemExit('short frame')
    declared = struct.unpack('<I', data[:4])[0]
    body = data[4:]
    if len(body) != declared:
        raise SystemExit('frame length mismatch')
    return json.loads(body.decode())


def send_frame(payload):
    body = json.dumps(payload, separators=(',', ':')).encode()
    conn.sendall(struct.pack('<I', len(body)) + body)

hello = recv_frame()
if hello.get('type') != 'hello':
    raise SystemExit(f'unexpected hello frame: {hello!r}')
send_frame({
    'type': 'helloOk',
    'serverVersion': '0.4.0',
    'selectedVersion': '0.4.0',
    'capabilities': ['typed-errors', 'export-broker-audit'],
})
request = recv_frame()
if request.get('type') != 'audit':
    raise SystemExit(f'unexpected audit frame: {request!r}')
if mode == 'success':
    send_frame({
        'type': 'auditResponse',
        'lines': ['broker audit line 1', 'broker audit line 2'],
    })
else:
    send_frame({
        'type': 'error',
        'error': {
            'kind': 'authz-audit-requires-admin',
            'exitCode': 32,
            'message': 'audit requires an admin role from nixling.site.adminUsers',
            'remediation': 'add the caller to nixling.site.adminUsers to use audit',
        },
    })
conn.close()
server.close()
PY

python3 "$scratch/mock-daemon.py" "$scratch/mock.sock" success > "$scratch/mock-daemon.log" 2>&1 &
mock_pid=$!
wait_for_socket "$scratch/mock.sock"
NIXLING_PUBLIC_SOCKET="$scratch/mock.sock" \
  "$cli" audit --human > "$scratch/mock-audit.human" 2> "$scratch/mock-audit.stderr"
wait "$mock_pid"
mock_pid=

if cmp -s "$scratch/mock-audit.human" <(printf 'broker audit line 1\nbroker audit line 2\n') \
  && [ ! -s "$scratch/mock-audit.stderr" ]; then
  ok "audit parses daemon auditResponse frames without falling back"
else
  fail "daemon auditResponse handling regressed"
  exit 1
fi

daemon_bin=$(nl_daemon_native_bin)
socket_path="$scratch/run/public.sock"
state_lock="$scratch/run/daemon.lock"
locks_dir="$scratch/run/locks"
daemon_state_dir="$scratch/run/daemon-state"
config_json="$scratch/run/config.json"
mkdir -p "$scratch/run" "$daemon_state_dir"
cat > "$config_json" <<EOF2
{
  "publicSocketPath": "$socket_path",
  "brokerSocketPath": "$scratch/run/priv.sock",
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
EOF2

(
  export NIXLINGD_TEST_PEER_UID=60003
  export NIXLINGD_TEST_PEER_GID=60003
  export NIXLINGD_TEST_PEER_USERNAME=launcher-user
  export NIXLINGD_TEST_PEER_GROUPS=wheel
  export NIXLING_SKIP_KERNEL_MODULE_CHECK=1
  "$daemon_bin" serve \
    --config "$config_json" \
    --test-listen-on "$socket_path" \
    --state-lock "$state_lock" \
    --locks-dir "$locks_dir" \
    --daemon-state-dir "$daemon_state_dir" \
    --once \
    --allow-unprivileged-runtime-dir \
    --no-drop-privileges
) > "$scratch/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_socket "$socket_path"

set +e
NIXLING_PUBLIC_SOCKET="$socket_path" \
  "$cli" audit --json > "$scratch/admin-rejected.stdout" 2> "$scratch/admin-rejected.stderr"
rc_admin=$?
set -e
wait "$daemon_pid"
daemon_pid=

[ "$rc_admin" -eq 32 ] || { fail "daemon-reachable admin-rejected case should return exit 32"; exit 1; }
if grep -Fq 'authz-audit-requires-admin' "$scratch/admin-rejected.stderr" \
  && ! grep -Fq 'daemon-down' "$scratch/admin-rejected.stderr" \
  && [ ! -s "$scratch/admin-rejected.stdout" ]; then
  ok "audit surfaces daemon authz errors without falling back"
else
  fail "daemon authz rejection handling regressed"
  exit 1
fi

set +e
NIXLING_PUBLIC_SOCKET="$scratch/strict.sock" \
  "$cli" audit --strict --json > "$scratch/strict.stdout" 2> "$scratch/strict.stderr"
rc_strict=$?
set -e

[ "$rc_strict" -eq 78 ] || { fail "audit --strict should return not-yet-implemented exit 78"; exit 1; }
if jq -e '.code == "not-yet-implemented" and .exitCode == 78' "$scratch/strict.stdout" >/dev/null 2>&1 \
  && [ ! -s "$scratch/strict.stderr" ]; then
  ok "audit --strict returns typed not-yet-implemented envelope"
else
  fail "audit --strict typed envelope regressed"
  exit 1
fi

log "==> cli-rust-native-audit OK"
