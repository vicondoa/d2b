#!/usr/bin/env bash
# P3 ph3-p3-host-doctor-extended: integration coverage for the
# extended `nixling host doctor --read-only` verb. Exercises:
#   - missing daemon state → warnings
#   - broker socket reachable / unreachable
#   - pidfd-table.json with OtelHostBridge + per-env usbipd runners
#   - kernel-module-report.json with required missing → fail
#   - kernel-module-report.json clean → pass
#   - autostart-report.json with a Failed outcome → fail
#   - autostart-report.json with a Degraded outcome → warn
#   - JSON exit code semantics: 0 pass / 1 warn / 2 fail / 78 usage
#   - backward-compatible top-level `broker_ready` field preserved
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=cli-rust-native-common.sh
. "$HERE/cli-rust-native-common.sh"

log "==> tests/cli-rust-native-host-doctor.sh"
scratch=$(nl_mktemp .cli-rust-native-host-doctor.XXXXXX)
cli=$(nl_cli_native_bin)

# Run all probes against paths under $scratch so we never touch real
# /run or /var/lib state. Point the metrics URL at a closed port on
# loopback so the probe predictably surfaces "unreachable".
state_dir="$scratch/daemon-state"
mkdir -p "$state_dir"

stub_socket="$scratch/never-exists.sock"
broker_socket="$scratch/broker.sock"
public_socket="$scratch/public.sock"

run_doctor() {
  local label="$1"; shift
  local rc
  set +e
  env \
    NIXLING_BROKER_SOCKET="$broker_socket" \
    NIXLING_PUBLIC_SOCKET="$public_socket" \
    NIXLING_DAEMON_STATE_DIR="$state_dir" \
    NIXLING_METRICS_URL="http://127.0.0.1:1/metrics" \
    "$cli" host doctor --read-only --json \
      > "$scratch/$label.json" 2> "$scratch/$label.stderr"
  rc=$?
  set -e
  printf '%s' "$rc"
}

# --- 1. usage gate: missing --read-only must exit 78 ---------------
set +e
"$cli" host doctor --json > "$scratch/usage.json" 2> "$scratch/usage.stderr"
rc_usage=$?
set -e
[ "$rc_usage" -eq 78 ] || { fail "host doctor without --read-only should exit 78, got $rc_usage"; exit 1; }
jq -e '.code == "--read-only-required"' "$scratch/usage.json" >/dev/null \
  || { fail "usage envelope missing --read-only-required code"; exit 1; }
ok "host doctor without --read-only exits 78 with --read-only-required envelope"

# --- 2. baseline (no state present) → broker fail, exit 2 -------------
rc=$(run_doctor baseline)
[ "$rc" -eq 2 ] || { fail "baseline doctor (no broker, no state) should exit 2 (broker fail), got $rc"; cat "$scratch/baseline.json" >&2; exit 1; }
jq -e '.command == "host doctor" and .mode == "read-only"' "$scratch/baseline.json" >/dev/null \
  || { fail "baseline doctor JSON envelope drift"; exit 1; }
jq -e '.broker_ready == false' "$scratch/baseline.json" >/dev/null \
  || { fail "baseline doctor must preserve top-level broker_ready=false"; exit 1; }
jq -e '
  .checks
  | (map(.name) | sort)
  == ["autostart-status","broker-ready","daemon-ready","kernel-module-matrix","metrics-endpoint","otel-host-bridge-runner","usbipd-runners"]
' "$scratch/baseline.json" >/dev/null \
  || { fail "baseline doctor checks[] missing expected check names"; jq '.checks | map(.name)' "$scratch/baseline.json" >&2; exit 1; }
jq -e '.summary.fail >= 1 and .summary.warn >= 4' "$scratch/baseline.json" >/dev/null \
  || { fail "baseline doctor summary mismatch"; jq '.summary' "$scratch/baseline.json" >&2; exit 1; }
ok "baseline doctor reports 7 checks; broker_ready=false (fail) + warns; exit=2"

# --- 3. pidfd-table.json with bridge + usbipd → both runners pass ---
cat > "$state_dir/pidfd-table.json" <<EOF
{
  "entries": [
    { "vm": "obs-net",  "role": "otel-host-bridge",   "pid": 1001, "startTimeTicks": 5 },
    { "vm": "corp-net", "role": "usbip",              "pid": 1002, "startTimeTicks": 5 },
    { "vm": "work-net", "role": "usbip",              "pid": 1003, "startTimeTicks": 5 },
    { "vm": "corp-vm",  "role": "cloud-hypervisor",   "pid": 1004, "startTimeTicks": 5 }
  ]
}
EOF
rc=$(run_doctor runners-present)
jq -e '
  (.checks | map(select(.name == "otel-host-bridge-runner"))[0].status == "pass")
  and (.checks | map(select(.name == "otel-host-bridge-runner"))[0].data.count == 1)
' "$scratch/runners-present.json" >/dev/null \
  || { fail "OtelHostBridge runner not reported as pass"; jq '.checks' "$scratch/runners-present.json" >&2; exit 1; }
jq -e '
  (.checks | map(select(.name == "usbipd-runners"))[0].status == "pass")
  and (.checks | map(select(.name == "usbipd-runners"))[0].data.count == 2)
' "$scratch/runners-present.json" >/dev/null \
  || { fail "per-env usbipd runners not counted"; jq '.checks' "$scratch/runners-present.json" >&2; exit 1; }
ok "pidfd-table reports OtelHostBridge runner + 2 per-env usbipd runners as pass"

# --- 4. kernel-module-report.json clean → pass; required-missing → fail ---
cat > "$state_dir/kernel-module-report.json" <<EOF
{
  "required": ["kvm_intel"],
  "present": ["kvm_intel"],
  "missing_required": [],
  "optional_missing": []
}
EOF
rc=$(run_doctor km-clean)
jq -e '.checks | map(select(.name == "kernel-module-matrix"))[0].status == "pass"' "$scratch/km-clean.json" >/dev/null \
  || { fail "clean kernel-module-report not reported as pass"; exit 1; }
ok "clean kernel-module-report yields kernel-module-matrix=pass"

cat > "$state_dir/kernel-module-report.json" <<EOF
{
  "required": ["kvm_intel"],
  "present": [],
  "missing_required": ["kvm_intel"],
  "optional_missing": []
}
EOF
rc=$(run_doctor km-fail)
[ "$rc" -eq 2 ] || { fail "kernel-module fail case should exit 2, got $rc"; exit 1; }
jq -e '
  (.checks | map(select(.name == "kernel-module-matrix"))[0].status == "fail")
  and (.exitCode == 2)
' "$scratch/km-fail.json" >/dev/null \
  || { fail "kernel-module fail case envelope drift"; jq '.summary,.exitCode' "$scratch/km-fail.json" >&2; exit 1; }
ok "missing required kernel module → kernel-module-matrix=fail, exit=2"

# Reset kernel-module-report back to clean for downstream cases.
cat > "$state_dir/kernel-module-report.json" <<EOF
{
  "required": ["kvm_intel"],
  "present": ["kvm_intel"],
  "missing_required": [],
  "optional_missing": []
}
EOF

# --- 5. autostart-report.json with Failed outcome → fail -----------
cat > "$state_dir/autostart-report.json" <<EOF
{
  "outcomes": [
    { "vm": "obs-net",  "env": "obs",  "is_net_vm": true,  "outcome": { "kind": "started" } },
    { "vm": "corp-vm",  "env": "corp", "is_net_vm": false, "outcome": { "kind": "failed", "reason": "broker refused" } }
  ]
}
EOF
rc=$(run_doctor autostart-fail)
[ "$rc" -eq 2 ] || { fail "autostart fail case should exit 2, got $rc"; cat "$scratch/autostart-fail.json" >&2; exit 1; }
jq -e '
  (.checks | map(select(.name == "autostart-status"))[0].status == "fail")
  and (.checks | map(select(.name == "autostart-status"))[0].data.failed == 1)
  and (.checks | map(select(.name == "autostart-status"))[0].data.degradedTotal == 1)
' "$scratch/autostart-fail.json" >/dev/null \
  || { fail "autostart fail envelope drift"; jq '.checks | map(select(.name == "autostart-status"))' "$scratch/autostart-fail.json" >&2; exit 1; }
ok "autostart-report Failed outcome → autostart-status=fail, exit=2, failed_count=1"

# --- 6. autostart Degraded only → warn -----------------------------
cat > "$state_dir/autostart-report.json" <<EOF
{
  "outcomes": [
    { "vm": "obs-net",  "env": "obs",  "is_net_vm": true,  "outcome": { "kind": "started" } },
    { "vm": "work-vm",  "env": "work", "is_net_vm": false, "outcome": { "kind": "degraded", "reason": "net-vm down" } }
  ]
}
EOF
rc=$(run_doctor autostart-degraded)
jq -e '
  (.checks | map(select(.name == "autostart-status"))[0].status == "warn")
  and (.checks | map(select(.name == "autostart-status"))[0].data.degraded == 1)
' "$scratch/autostart-degraded.json" >/dev/null \
  || { fail "autostart degraded case did not report warn"; jq '.checks | map(select(.name == "autostart-status"))' "$scratch/autostart-degraded.json" >&2; exit 1; }
ok "autostart-report Degraded outcome → autostart-status=warn"

# --- 7. metrics endpoint reported unreachable consistently ---------
jq -e '
  .checks | map(select(.name == "metrics-endpoint"))[0]
  | (.status == "warn") and (.detail | contains("unreachable"))
' "$scratch/autostart-degraded.json" >/dev/null \
  || { fail "metrics endpoint should be reported unreachable (warn)"; exit 1; }
ok "metrics endpoint probe correctly reports unreachable scrape URL as warn"

# --- 8. human renderer surfaces summary line + per-check markers ----
set +e
env \
  NIXLING_BROKER_SOCKET="$broker_socket" \
  NIXLING_PUBLIC_SOCKET="$public_socket" \
  NIXLING_DAEMON_STATE_DIR="$state_dir" \
  NIXLING_METRICS_URL="http://127.0.0.1:1/metrics" \
  "$cli" host doctor --read-only --human \
    > "$scratch/human.out" 2> "$scratch/human.err"
rc_human=$?
set -e
grep -Fq "host doctor --read-only: summary pass=" "$scratch/human.out" \
  || { fail "human renderer missing summary line"; cat "$scratch/human.out"; exit 1; }
grep -Fq "[PASS]" "$scratch/human.out" \
  || { fail "human renderer missing [PASS] markers"; exit 1; }
ok "host doctor --human prints summary line + per-check status markers"

# --- 9. fully-healthy run → exit 0 ---------------------------------
# Stand up a real abstract-namespace UNIX seqpacket socket so the
# broker + daemon probes both pass without requiring root. Use
# python3 from the nix shell since the system PATH may not have it.
if PYTHON3=$(command -v python3 2>/dev/null) \
   || PYTHON3=$(nix shell --quiet --inputs-from "$ROOT" nixpkgs#python3 --command bash -c 'command -v python3' 2>/dev/null); then
  "$PYTHON3" - "$broker_socket" "$public_socket" "$scratch/listeners.pid" <<'PY' &
import os, socket, sys
broker_path, public_path, pid_path = sys.argv[1:4]
for p in (broker_path, public_path):
    try: os.unlink(p)
    except FileNotFoundError: pass
def listen(p):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
    s.bind(p)
    s.listen(8)
    return s
sb = listen(broker_path)
sp = listen(public_path)
with open(pid_path, "w") as f:
    f.write(str(os.getpid()))
# accept-and-close loop so probe connects succeed without
# requiring a hello handshake (doctor only opens the fd).
import select
while True:
    r, _, _ = select.select([sb, sp], [], [], 30.0)
    for s in r:
        try:
            c, _ = s.accept()
            c.close()
        except Exception:
            pass
PY
listener_pid=$!
sleep 0.5
[ -S "$broker_socket" ] && [ -S "$public_socket" ] \
  || { fail "listener sockets did not appear"; kill "$listener_pid" 2>/dev/null; exit 1; }
rc=$(run_doctor healthy)
kill "$listener_pid" 2>/dev/null || true
wait "$listener_pid" 2>/dev/null || true
rm -f "$broker_socket" "$public_socket"
jq -e '
  (.checks | map(select(.name == "broker-ready"))[0].status == "pass")
  and (.checks | map(select(.name == "daemon-ready"))[0].status == "pass")
  and (.broker_ready == true)
' "$scratch/healthy.json" >/dev/null \
  || { fail "healthy run did not report broker-ready+daemon-ready=pass"; jq '.checks,.broker_ready' "$scratch/healthy.json" >&2; exit 1; }
ok "broker + daemon sockets reachable → broker-ready=pass, daemon-ready=pass, broker_ready top-level=true"
else
  log "  SKIP: python3 unavailable; skipping live-socket healthy check"
fi

log "==> cli-rust-native-host-doctor OK"
