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
legacy=$(nl_legacy_cli_bin)

NIXLING_LEGACY_CLI="$legacy" \
NIXLING_PUBLIC_SOCKET="$scratch/missing.sock" \
NIXLING_AUDIT_TESTMODE_KVM_MODE=660 \
  "$cli" audit --human > "$scratch/audit.human" 2> "$scratch/audit.human.stderr"
NIXLING_LEGACY_CLI="$legacy" \
NIXLING_PUBLIC_SOCKET="$scratch/missing.sock" \
NIXLING_AUDIT_TESTMODE_KVM_MODE=660 \
  "$cli" audit --json > "$scratch/audit.json" 2> "$scratch/audit.json.stderr"

nl_assert_json_schema "$ROOT/docs/reference/cli-output/audit.schema.json" "$scratch/audit.json"
ok "audit --json validates against docs/reference/cli-output/audit.schema.json"

if grep -Fq 'v1.0 daemon-only: nixlingd unreachable' "$scratch/audit.json.stderr" \
  && grep -Fq 'v1.0 daemon-only: nixlingd unreachable' "$scratch/audit.human.stderr"; then
  ok "audit fallback warns on stderr when nixlingd is unreachable"
else
  fail "audit fallback warning missing"
  exit 1
fi

if jq -e '.kvm_dev_mode == "660" and .bridge_isolation["corp-vm"].isolated == true' "$scratch/audit.json" >/dev/null 2>&1; then
  ok "audit --json preserves the legacy bash contract on fallback"
else
  fail "audit --json output regressed"
  exit 1
fi

if grep -Fq '=== nixling security audit ===' "$scratch/audit.human"; then
  ok "audit --human preserves the legacy human report on fallback"
else
  fail "audit --human output regressed"
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
NIXLING_LEGACY_CLI="$legacy" \
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
config_json="$scratch/run/config.json"
mkdir -p "$scratch/run"
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
  "$daemon_bin" serve \
    --config "$config_json" \
    --test-listen-on "$socket_path" \
    --state-lock "$state_lock" \
    --locks-dir "$locks_dir" \
    --once \
    --allow-unprivileged-runtime-dir \
    --no-drop-privileges
) > "$scratch/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_socket "$socket_path"

set +e
NIXLING_LEGACY_CLI="$legacy" \
NIXLING_PUBLIC_SOCKET="$socket_path" \
  "$cli" audit --json > "$scratch/admin-rejected.stdout" 2> "$scratch/admin-rejected.stderr"
rc_admin=$?
set -e
wait "$daemon_pid"
daemon_pid=

[ "$rc_admin" -eq 32 ] || { fail "daemon-reachable admin-rejected case should return exit 32"; exit 1; }
if grep -Fq 'authz-audit-requires-admin' "$scratch/admin-rejected.stderr" \
  && ! grep -Fq 'v1.0 daemon-only: nixlingd unreachable' "$scratch/admin-rejected.stderr" \
  && [ ! -s "$scratch/admin-rejected.stdout" ]; then
  ok "audit surfaces daemon authz errors surfacing v1.0 daemon-only exit-78 envelope"
else
  fail "daemon authz rejection handling regressed"
  exit 1
fi

cat > "$scratch/mock-legacy-audit.sh" <<'EOF2'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$@" > "$MOCK_ARGV_FILE"
printf '{"strict":true}\n'
exit 41
EOF2
chmod +x "$scratch/mock-legacy-audit.sh"

set +e
MOCK_ARGV_FILE="$scratch/strict-argv.txt" \
NIXLING_LEGACY_CLI="$scratch/mock-legacy-audit.sh" \
NIXLING_PUBLIC_SOCKET="$scratch/strict.sock" \
  "$cli" audit --strict --json > "$scratch/strict.stdout" 2> "$scratch/strict.stderr"
rc_strict=$?
set -e

[ "$rc_strict" -eq 41 ] || { fail "audit --strict should preserve the legacy bash exit code"; exit 1; }
if cmp -s "$scratch/strict-argv.txt" <(printf 'audit\n--strict\n--json\n') \
  && cmp -s "$scratch/strict.stdout" <(printf '{"strict":true}\n') \
  && ! grep -Fq 'unknown argument' "$scratch/strict.stderr"; then
  ok "audit --strict dispatches to legacy bash instead of failing clap parsing"
else
  fail "audit --strict compatibility regressed"
  exit 1
fi

log "==> cli-rust-native-audit OK"
