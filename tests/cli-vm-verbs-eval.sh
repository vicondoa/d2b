#!/usr/bin/env bash
# tests/cli-vm-verbs-eval.sh— cli-up layer-1 gate.
#
# Asserts the Rust CLI's `up/down/restart/list` verbs (and their
# `vm start/stop/restart/list` aliases) are fully daemon-native:
#
#   1. With nixlingd's public socket missing, every mutating verb
#      surfaces the typed `daemon-down` envelope and exits 1.
#      No bash fallback is attempted, even when the (now-removed)
#      `NIXLING_LEGACY_BASH_OPT_IN=1` escape hatch is set.
#   2. The `NIXLING_LEGACY_CLI_PATH` poison-pill is never invoked
#      (proven by routing it through a non-executable sentinel that
#      would `exit 99` if ever exec'd).
#   3. `vm list` returns the rust-native JSON envelope without
#      touching bash.
#
# Layer 1 (no live daemon, no microvm spawn). Runs in seconds.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=cli-rust-native-common.sh
. "$HERE/cli-rust-native-common.sh"

log "==> tests/cli-vm-verbs-eval.sh"

cli=$(nl_cli_native_bin)
scratch=$(nl_mktemp .cli-vm-verbs-eval.XXXXXX)

manifest="$scratch/vms.json"
cat > "$manifest" <<'JSON'
{
  "test-vm": {
    "name": "test-vm",
    "env": "work",
    "graphics": false,
    "tpm": false,
    "audio": false,
    "audioService": "none",
    "usbipYubikey": false,
    "staticIp": null,
    "isNetVm": false,
    "stateDir": "/var/lib/nixling/vms/test-vm",
    "bridge": "nl-work",
    "sshUser": null
  }
}
JSON

# A poison-pill "legacy bash CLI": if the rust CLI ever exec's it we
# fail the assertion with a distinctive exit code.
poison="$scratch/legacy-poison.sh"
cat > "$poison" <<'POISON'
#!/usr/bin/env bash
echo "FAIL: rust CLI exec'd the legacy bash poison-pill with args: $*" >&2
exit 99
POISON
chmod +x "$poison"

socket_missing="$scratch/never-bound.sock"
[ -e "$socket_missing" ] && rm -f -- "$socket_missing"

run_verb() {
  local label="$1"
  shift
  local out="$scratch/$label.out"
  local err="$scratch/$label.err"
  local rc=0
  NIXLING_MANIFEST_PATH="$manifest" \
  NIXLING_PUBLIC_SOCKET="$socket_missing" \
  NIXLING_LEGACY_CLI_PATH="$poison" \
  NIXLING_LEGACY_BASH_OPT_IN=1 \
  NIXLING_SUPPRESS_LEGACY_BASH_WARNING=1 \
    "$cli" "$@" > "$out" 2> "$err" || rc=$?
  printf '%s\n' "$rc"
}

assert_daemon_down_envelope() {
  local label="$1"
  local rc="$2"
  local out="$scratch/$label.out"
  local err="$scratch/$label.err"
  if [ "$rc" = "99" ]; then
    fail "$label: rust CLI exec'd the legacy bash poison-pill (NIXLING_LEGACY_BASH_OPT_IN must NOT be honoured)"
    cat "$err" >&2
    exit 1
  fi
  if [ "$rc" = "0" ]; then
    fail "$label: expected daemon-down typed envelope (exit 1), got success"
    cat "$out" "$err" >&2
    exit 1
  fi
  # JSON envelopes go to stdout (emit_host_error in --json mode);
  # rendered text envelopes go to stderr. Accept either.
  for stream in "$out" "$err"; do
    if jq -e '.code == "daemon-down"' "$stream" >/dev/null 2>&1; then
      ok "$label: emitted typed daemon-down envelope (exit $rc, no bash fallback)"
      return 0
    fi
  done
  if grep -qE 'daemon-down|Daemon required' "$out" "$err"; then
    ok "$label: emitted typed daemon-down envelope (text form, exit $rc)"
    return 0
  fi
  fail "$label: expected daemon-down envelope"
  echo "--- stdout ---" >&2
  cat "$out" >&2
  echo "--- stderr ---" >&2
  cat "$err" >&2
  exit 1
}

# --- (1) typed-error path for every mutating verb -------------------

for verb_pair in \
  "up:up test-vm --apply --json" \
  "down:down test-vm --apply --json" \
  "restart:restart test-vm --apply --json" \
  "vm-start:vm start test-vm --apply --json" \
  "vm-stop:vm stop test-vm --apply --json" \
  "vm-restart:vm restart test-vm --apply --json" \
; do
  label="${verb_pair%%:*}"
  argv="${verb_pair#*:}"
  # shellcheck disable=SC2086
  rc=$(run_verb "$label" $argv)
  assert_daemon_down_envelope "$label" "$rc"
done

# --- (2) NIXLING_LEGACY_BASH_OPT_IN is dead --------------------------

# Already covered by run_verb above: every call exported
# NIXLING_LEGACY_BASH_OPT_IN=1 with a poison-pill legacy path. No
# verb reached the poison-pill (rc != 99). Re-assert via a direct
# spot-check.
rc=$(run_verb "opt-in-dead" up test-vm --apply --json)
if [ "$rc" = "99" ]; then
  fail "NIXLING_LEGACY_BASH_OPT_IN was honoured — escape hatch must be removed"
  exit 1
fi
ok "NIXLING_LEGACY_BASH_OPT_IN=1 is a no-op (escape hatch removed)"

# --- (3) vm list is daemon-native (no bash exec) --------------------

list_out="$scratch/vm-list.out"
list_err="$scratch/vm-list.err"
list_rc=0
NIXLING_MANIFEST_PATH="$manifest" \
NIXLING_PUBLIC_SOCKET="$socket_missing" \
NIXLING_LEGACY_CLI_PATH="$poison" \
NIXLING_LEGACY_BASH_OPT_IN=1 \
NIXLING_SUPPRESS_LEGACY_BASH_WARNING=1 \
  "$cli" vm list --json > "$list_out" 2> "$list_err" || list_rc=$?
if [ "$list_rc" = "99" ]; then
  fail "vm list exec'd the legacy bash poison-pill"
  cat "$list_err" >&2
  exit 1
fi
if [ "$list_rc" != "0" ]; then
  fail "vm list expected exit 0, got $list_rc"
  cat "$list_err" >&2
  exit 1
fi
if ! jq -e '.command == "vm list"' "$list_out" >/dev/null; then
  fail "vm list did not emit the rust-native JSON envelope"
  cat "$list_out" >&2
  exit 1
fi
ok "vm list returns native rust JSON without bash fallback"

# --- (4) top-level `list` is also native ---------------------------

# `nixling list` is the manifest view and was already native, but
# re-assert with the same poison-pill setup to keep the gate honest.
top_list_out="$scratch/top-list.out"
top_list_rc=0
NIXLING_MANIFEST_PATH="$manifest" \
NIXLING_PUBLIC_SOCKET="$socket_missing" \
NIXLING_LEGACY_CLI_PATH="$poison" \
NIXLING_LEGACY_BASH_OPT_IN=1 \
NIXLING_SUPPRESS_LEGACY_BASH_WARNING=1 \
  "$cli" list --json > "$top_list_out" 2>/dev/null || top_list_rc=$?
if [ "$top_list_rc" = "99" ]; then
  fail "top-level list exec'd the legacy bash poison-pill"
  exit 1
fi
ok "nixling list is native (exit $top_list_rc, no bash fallback)"

# --- (5) vm konsole --dry-run --json shape (guest-control transport) ---
#
# `vm konsole` now hosts `nixling vm exec -it <vm> -- /run/current-system/sw/bin/bash -l` over the
# authenticated guest-control transport (no SSH). Asserts the --dry-run
# JSON shape without spawning a terminal or contacting a socket. Layer 1
# (no socket, no fork).

konsole_manifest="$scratch/konsole-manifest.json"
cat > "$konsole_manifest" <<'JSON'
{
  "konsole-vm": {
    "name": "konsole-vm",
    "env": "work",
    "graphics": false,
    "tpm": false,
    "audio": false,
    "audioService": "none",
    "usbipYubikey": false,
    "staticIp": "10.30.0.99",
    "isNetVm": false,
    "stateDir": "/var/lib/nixling/vms/konsole-vm",
    "bridge": "nl-work",
    "sshUser": "alice"
  }
}
JSON

konsole_out="$scratch/konsole-dry.json"
konsole_rc=0
NIXLING_MANIFEST_PATH="$konsole_manifest" \
NIXLING_PUBLIC_SOCKET="$socket_missing" \
  "$cli" vm konsole konsole-vm --dry-run --json > "$konsole_out" 2>/dev/null || konsole_rc=$?
if [ "$konsole_rc" != "0" ]; then
  fail "vm konsole --dry-run --json should exit 0, got $konsole_rc"
  cat "$konsole_out" >&2
  exit 1
fi

if command -v jq >/dev/null 2>&1; then
  cmd=$(jq -r '.command' "$konsole_out")
  mode=$(jq -r '.mode' "$konsole_out")
  vm=$(jq -r '.vm' "$konsole_out")
  terminal=$(jq -r '.terminal' "$konsole_out")
  transport=$(jq -r '.transport' "$konsole_out")
  argv0=$(jq -r '.argv[0]' "$konsole_out")
  argv_joined=$(jq -r '.argv | join(" ")' "$konsole_out")
  has_host=$(jq -r 'has("host")' "$konsole_out")
  has_user=$(jq -r 'has("user")' "$konsole_out")
  has_key=$(jq -r 'has("key")' "$konsole_out")
  [ "$cmd" = "vm konsole" ] || { fail "vm konsole .command='$cmd' (want 'vm konsole')"; exit 1; }
  [ "$mode" = "dry-run" ] || { fail "vm konsole .mode='$mode' (want 'dry-run')"; exit 1; }
  [ "$vm" = "konsole-vm" ] || { fail "vm konsole .vm='$vm' (want 'konsole-vm')"; exit 1; }
  [ "$terminal" = "konsole" ] || { fail "vm konsole .terminal='$terminal' (want 'konsole' default)"; exit 1; }
  [ "$transport" = "guest-control" ] || { fail "vm konsole .transport='$transport' (want 'guest-control')"; exit 1; }
  [ "$argv0" = "konsole" ] || { fail "vm konsole .argv[0]='$argv0' (want 'konsole' terminal)"; exit 1; }
  # konsole hosts `nixling vm exec -it <vm> -- bash -l` over guest-control.
  case "$argv_joined" in
    *"vm exec -it konsole-vm -- /run/current-system/sw/bin/bash -l") : ;;
    *) fail "vm konsole .argv must host 'vm exec -it konsole-vm -- /run/current-system/sw/bin/bash -l' (got '$argv_joined')"; exit 1 ;;
  esac
  # The retired SSH fields must be absent from the JSON entirely.
  [ "$has_host" = "false" ] || { fail "vm konsole must not emit SSH .host"; exit 1; }
  [ "$has_user" = "false" ] || { fail "vm konsole must not emit SSH .user"; exit 1; }
  [ "$has_key" = "false" ] || { fail "vm konsole must not emit SSH .key"; exit 1; }
fi
ok "vm konsole --dry-run --json emits the guest-control transport shape"

# --- (5b) vm konsole --terminal override reflected; retired SSH flags rejected ---

konsole_override_out="$scratch/konsole-override.json"
NIXLING_MANIFEST_PATH="$konsole_manifest" \
NIXLING_PUBLIC_SOCKET="$socket_missing" \
  "$cli" vm konsole konsole-vm \
    --dry-run --json \
    --terminal xterm \
  > "$konsole_override_out" 2>/dev/null
if command -v jq >/dev/null 2>&1; then
  terminal=$(jq -r '.terminal' "$konsole_override_out")
  argv0=$(jq -r '.argv[0]' "$konsole_override_out")
  [ "$terminal" = "xterm" ] || { fail "vm konsole --terminal override not reflected (got '$terminal')"; exit 1; }
  [ "$argv0" = "xterm" ] || { fail "vm konsole --terminal not reflected in argv[0] (got '$argv0')"; exit 1; }
fi
ok "vm konsole --terminal override reflected in dry-run JSON"

# Retired SSH-only flags (--host/--user/--key) must be rejected with a clear
# non-zero exit and a migration message, not silently honored.
retired_host_rc=0
NIXLING_MANIFEST_PATH="$konsole_manifest" \
NIXLING_PUBLIC_SOCKET="$socket_missing" \
  "$cli" vm konsole konsole-vm --dry-run --json --host 192.0.2.44 > /dev/null 2>&1 || retired_host_rc=$?
[ "$retired_host_rc" != "0" ] || { fail "vm konsole must reject retired --host (exited 0)"; exit 1; }

retired_user_rc=0
NIXLING_MANIFEST_PATH="$konsole_manifest" \
NIXLING_PUBLIC_SOCKET="$socket_missing" \
  "$cli" vm konsole konsole-vm --dry-run --json --user bob > /dev/null 2>&1 || retired_user_rc=$?
[ "$retired_user_rc" != "0" ] || { fail "vm konsole must reject retired --user (exited 0)"; exit 1; }

retired_key_rc=0
NIXLING_MANIFEST_PATH="$konsole_manifest" \
NIXLING_PUBLIC_SOCKET="$socket_missing" \
  "$cli" vm konsole konsole-vm --dry-run --json --key /custom/key > /dev/null 2>&1 || retired_key_rc=$?
[ "$retired_key_rc" != "0" ] || { fail "vm konsole must reject retired --key (exited 0)"; exit 1; }
ok "vm konsole rejects retired SSH-only flags (--host/--user/--key)"

# --- (5c) vm konsole unknown VM exits 1 with clear error ---

konsole_unknown_rc=0
NIXLING_MANIFEST_PATH="$konsole_manifest" \
NIXLING_PUBLIC_SOCKET="$socket_missing" \
  "$cli" vm konsole missing-vm --dry-run --json > /dev/null 2>&1 || konsole_unknown_rc=$?
if [ "$konsole_unknown_rc" = "0" ]; then
  fail "vm konsole on unknown vm should exit 1, got 0"
  exit 1
fi
ok "vm konsole rejects unknown vm name with non-zero exit"

# --- (6) vm exec black-box: clap parse + top-level dispatch ---
#
# Exercises `nixling vm exec` through real clap parsing and dispatch (not the
# Rust unit tests that build VmExecArgs directly). Hermetic: no daemon, no
# guest. Asserts (a) the cli/usage envelope for a missing command and (b) the
# guest-control-transport-unavailable envelope when the daemon socket is
# absent — proving the primary operator command reaches cmd_vm_exec.

exec_usage_out="$scratch/exec-usage.json"
exec_usage_rc=0
NIXLING_MANIFEST_PATH="$konsole_manifest" \
NIXLING_PUBLIC_SOCKET="$socket_missing" \
  "$cli" vm exec konsole-vm --json > "$exec_usage_out" 2>/dev/null || exec_usage_rc=$?
[ "$exec_usage_rc" = "2" ] || { fail "vm exec (no command) should exit 2, got $exec_usage_rc"; cat "$exec_usage_out" >&2; exit 1; }
if command -v jq >/dev/null 2>&1; then
  [ "$(jq -r '.command' "$exec_usage_out")" = "vm exec" ] || { fail "vm exec usage .command (want 'vm exec')"; exit 1; }
  [ "$(jq -r '.source' "$exec_usage_out")" = "cli" ] || { fail "vm exec usage .source (want 'cli')"; exit 1; }
  [ "$(jq -r '.reason' "$exec_usage_out")" = "usage" ] || { fail "vm exec usage .reason (want 'usage')"; exit 1; }
fi
ok "vm exec (missing command) emits the cli/usage envelope via clap+dispatch"

exec_transport_out="$scratch/exec-transport.json"
exec_transport_rc=0
NIXLING_MANIFEST_PATH="$konsole_manifest" \
NIXLING_PUBLIC_SOCKET="$socket_missing" \
  "$cli" vm exec konsole-vm --json -- /bin/true > "$exec_transport_out" 2>/dev/null || exec_transport_rc=$?
[ "$exec_transport_rc" != "0" ] || { fail "vm exec with no daemon should exit non-zero"; exit 1; }
if command -v jq >/dev/null 2>&1; then
  [ "$(jq -r '.command' "$exec_transport_out")" = "vm exec" ] || { fail "vm exec transport .command (want 'vm exec')"; exit 1; }
  [ "$(jq -r '.source' "$exec_transport_out")" = "transport" ] || { fail "vm exec transport .source (want 'transport')"; exit 1; }
  [ "$(jq -r '.reason' "$exec_transport_out")" = "guest-control-transport-unavailable" ] || { fail "vm exec transport .reason (want 'guest-control-transport-unavailable')"; exit 1; }
fi
ok "vm exec (no daemon) emits guest-control-transport-unavailable via clap+dispatch"

# -i/--interactive without -t/--tty is rejected (guestd forwards stdin only in
# PTY mode); the CLI must fail-fast with a usage error, not create a
# stdin-closed exec it then writes to.
exec_i_rc=0
exec_i_err="$scratch/exec-i.err"
NIXLING_MANIFEST_PATH="$konsole_manifest" \
NIXLING_PUBLIC_SOCKET="$socket_missing" \
  "$cli" vm exec konsole-vm -i -- /bin/true >/dev/null 2>"$exec_i_err" || exec_i_rc=$?
[ "$exec_i_rc" = "2" ] || { fail "vm exec -i without -t should exit 2, got $exec_i_rc"; cat "$exec_i_err" >&2; exit 1; }
grep -qiE 'requires .*-t/--tty|requires -t' "$exec_i_err" || { fail "vm exec -i without -t error must cite the -t/--tty requirement"; cat "$exec_i_err" >&2; exit 1; }
ok "vm exec rejects -i without -t (stdin forwarding requires a PTY)"

log "==> cli-vm-verbs-eval OK"
