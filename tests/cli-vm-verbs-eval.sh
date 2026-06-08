#!/usr/bin/env bash
# tests/cli-vm-verbs-eval.sh — P4 cli-up layer-1 gate.
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

# --- (5) vm konsole --dry-run --json shape (v1.1.2 panel-test must-fix) ---
#
# Asserts the new `vm konsole <vm>` verb (v1.1.2fu14d) emits the
# documented JSON shape in --dry-run mode without requiring a real
# ssh key file or actually spawning a terminal. Layer 1 (no socket,
# no fork). Covers the panel-test + panel-software must-fix items.

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
  host=$(jq -r '.host' "$konsole_out")
  user=$(jq -r '.user' "$konsole_out")
  key=$(jq -r '.key' "$konsole_out")
  argv0=$(jq -r '.argv[0]' "$konsole_out")
  [ "$cmd" = "vm konsole" ] || { fail "vm konsole .command='$cmd' (want 'vm konsole')"; exit 1; }
  [ "$mode" = "dry-run" ] || { fail "vm konsole .mode='$mode' (want 'dry-run')"; exit 1; }
  [ "$vm" = "konsole-vm" ] || { fail "vm konsole .vm='$vm' (want 'konsole-vm')"; exit 1; }
  [ "$terminal" = "konsole" ] || { fail "vm konsole .terminal='$terminal' (want 'konsole' default)"; exit 1; }
  [ "$host" = "10.30.0.99" ] || { fail "vm konsole .host='$host' (want '10.30.0.99' from staticIp)"; exit 1; }
  [ "$user" = "alice" ] || { fail "vm konsole .user='$user' (want 'alice' from sshUser)"; exit 1; }
  [ -n "$key" ] || { fail "vm konsole .key is empty"; exit 1; }
  [ "$argv0" = "konsole" ] || { fail "vm konsole .argv[0]='$argv0' (want 'konsole')"; exit 1; }
fi
ok "vm konsole --dry-run --json shape matches v1.1.2 contract"

# --- (5b) vm konsole overrides reflected in dry-run output ---

konsole_override_out="$scratch/konsole-override.json"
NIXLING_MANIFEST_PATH="$konsole_manifest" \
NIXLING_PUBLIC_SOCKET="$socket_missing" \
  "$cli" vm konsole konsole-vm \
    --dry-run --json \
    --user bob --host 192.0.2.44 --key /custom/key --terminal xterm \
  > "$konsole_override_out" 2>/dev/null
if command -v jq >/dev/null 2>&1; then
  user=$(jq -r '.user' "$konsole_override_out")
  host=$(jq -r '.host' "$konsole_override_out")
  key=$(jq -r '.key' "$konsole_override_out")
  terminal=$(jq -r '.terminal' "$konsole_override_out")
  [ "$user" = "bob" ] || { fail "vm konsole --user override not reflected (got '$user')"; exit 1; }
  [ "$host" = "192.0.2.44" ] || { fail "vm konsole --host override not reflected (got '$host')"; exit 1; }
  [ "$key" = "/custom/key" ] || { fail "vm konsole --key override not reflected (got '$key')"; exit 1; }
  [ "$terminal" = "xterm" ] || { fail "vm konsole --terminal override not reflected (got '$terminal')"; exit 1; }
fi
ok "vm konsole CLI overrides reflected in dry-run JSON"

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

log "==> cli-vm-verbs-eval OK"
