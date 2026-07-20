#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}
# shellcheck source=tests/cli-rust-native-common.sh
. "$ROOT/tests/cli-rust-native-common.sh"

log "==> tests/unit/gates/performance-budgets.sh"

if [ "${D2B_PERF_STABLE:-0}" != 1 ]; then
  log "SKIP: set D2B_PERF_STABLE=1 on a pinned self-hosted runner to enforce performance budgets"
  exit 0
fi

if [ -z "${D2B_PERF_IN_NIX_SHELL:-}" ] && ! command -v python3 >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "performance budgets require python3 (or nix to provide it) once D2B_PERF_STABLE=1 is set"
    exit 1
  fi
  export D2B_PERF_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" nixpkgs#python3 --command bash "$0" "$@"
fi

DAEMON_HELLO_BUDGET_MS=${D2B_PERF_DAEMON_HELLO_BUDGET_MS:-200}
BROKER_VALIDATE_P99_BUDGET_MS=${D2B_PERF_BROKER_VALIDATE_P99_BUDGET_MS:-25}
VM_START_DRY_RUN_BUDGET_MS=${D2B_PERF_VM_START_DRY_RUN_BUDGET_MS:-100}
IDENTITY_DERIVATION_BUDGET_MS=${D2B_PERF_IDENTITY_DERIVATION_BUDGET_MS:-5000}
IDENTITY_DERIVATION_RSS_KIB=${D2B_PERF_IDENTITY_DERIVATION_RSS_KIB:-524288}
MARGIN_PERCENT=20

scratch=$(d2b_mktemp .performance-budgets.XXXXXX)
add_cleanup "rm -rf -- \"$scratch\""
bundle_root=$(d2b_cli_smoke_bundle_tree)
cli_bin=$(d2b_cli_native_bin)
daemon_bin=$(d2b_daemon_native_bin)
broker_bin=$(d2b_cargo_bin_path broker d2b-priv-broker)
if [ ! -x "$broker_bin" ]; then
  d2b_cli_toolchain_shell "cd '$ROOT/packages' && CARGO_TARGET_DIR='$(d2b_cargo_target_dir workspace)' cargo build --locked -q --manifest-path '$ROOT/packages/Cargo.toml' -p d2b-priv-broker"
fi

wait_for_socket() {
  local path="$1"
  local attempts=0
  while [ "$attempts" -lt 300 ]; do
    [ -S "$path" ] && return 0
    attempts=$((attempts + 1))
    sleep 0.1
  done
  fail "timed out waiting for socket: $path"
  return 1
}

median_of_three() {
  printf '%s\n' "$1" "$2" "$3" | sort -n | sed -n '2p'
}

measure_identity_derivation() {
  local expression="$scratch/identity-benchmark.nix"
  cat >"$expression" <<EOF
let
  identity = import $ROOT/nixos-modules/v2-identity.nix;
  chains = builtins.genList
    (index:
      let
        suffix = builtins.toString index;
        realm = identity.deriveRealmId "realm-\${suffix}.local-root";
        workload = identity.deriveWorkloadId realm "workload-\${suffix}";
        provider = identity.deriveProviderId realm "runtime" "provider-\${suffix}";
        role = identity.deriveRoleId realm workload "cloud-hypervisor";
      in { inherit realm workload provider role; })
    1024;
  realms = map (chain: chain.realm) chains;
  workloads = map (chain: chain.workload) chains;
  providers = map (chain: chain.provider) chains;
  roles = map (chain: chain.role) chains;
  checked = identity.validateGlobalIdentities {
    inherit realms workloads providers roles;
  };
  allIds = realms ++ workloads ++ providers ++ roles;
in
builtins.deepSeq checked {
  count = builtins.length allIds;
  digest = builtins.hashString "sha256" (builtins.concatStringsSep "" allIds);
}
EOF

  python3 - "$expression" <<'PY'
import json
import resource
import subprocess
import sys
import time

expression = sys.argv[1]
command = [
    "nix", "eval", "--impure", "--json", "--file", expression,
]

def run_once():
    start = time.monotonic_ns()
    completed = subprocess.run(
        command,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=30,
    )
    elapsed_ms = (time.monotonic_ns() - start + 999_999) // 1_000_000
    result = json.loads(completed.stdout)
    if result.get("count") != 4096 or len(result.get("digest", "")) != 64:
        raise SystemExit("identity benchmark did not derive and check exactly 4,096 IDs")
    rss_kib = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    return elapsed_ms, rss_kib

run_once()
for _ in range(3):
    elapsed_ms, rss_kib = run_once()
    print(f"{elapsed_ms} {rss_kib}")
PY
}

measure_daemon_cold_start_ms() {
  local socket_path="$scratch/daemon/public.sock"
  local state_lock="$scratch/daemon/daemon.lock"
  local locks_dir="$scratch/daemon/locks"
  local config_json="$scratch/daemon/config.json"
  local sample1 sample2 sample3
  mkdir -p "$scratch/daemon"
  chmod 0755 "$scratch/daemon"
  cat >"$config_json" <<EOF
{
  "publicSocketPath": "$socket_path",
  "brokerSocketPath": "$scratch/daemon/priv.sock",
  "stateLockPath": "$state_lock",
  "locksDir": "$locks_dir",
  "daemonUser": "root",
  "daemonGroup": "root",
  "publicSocketGroup": "$(id -gn)",
  "launcherUsers": ["$(id -un)"],
  "adminUsers": [],
  "serverVersion": "0.4.0",
  "acceptedClientVersionRange": ">=0.4.0, <0.5.0"
}
EOF

  measure_once() {
    local start_ms end_ms daemon_pid attempts
    rm -f "$socket_path" "$state_lock"
    mkdir -p "$locks_dir"
    start_ms=$(d2b_now_ms)
    "$daemon_bin" serve \
      --config "$config_json" \
      --test-listen-on "$socket_path" \
      --state-lock "$state_lock" \
      --locks-dir "$locks_dir" \
      --once \
      --allow-unprivileged-runtime-dir \
      --no-drop-privileges \
      >"$scratch/daemon/serve.log" 2>&1 &
    daemon_pid=$!
    add_cleanup "kill $daemon_pid >/dev/null 2>&1 || true"
    attempts=0
    while [ "$attempts" -lt 300 ]; do
      if [ -S "$socket_path" ] && [ -f "$scratch/daemon/version" ]; then
        end_ms=$(d2b_now_ms)
        kill "$daemon_pid"
        wait "$daemon_pid" 2>/dev/null || true
        printf '%s\n' $((end_ms - start_ms))
        return 0
      fi
      attempts=$((attempts + 1))
      sleep 0.01
    done
    fail "daemon cold-start measurement timed out waiting for the first hello response"
    exit 1
  }

  sample1=$(measure_once)
  sample2=$(measure_once)
  sample3=$(measure_once)
  median_of_three "$sample1" "$sample2" "$sample3"
}

measure_broker_validate_p99_ms() {
  local socket_path="$scratch/broker/priv.sock"
  local audit_dir="$scratch/broker/audit"
  local broker_pid
  mkdir -p "$scratch/broker" "$audit_dir"
  rm -f "$socket_path"
  (
    "$broker_bin" serve \
      --socket-path "$socket_path" \
      --audit-dir "$audit_dir" \
      --audit-retention-days 0 \
      --bundle-path "$bundle_root/vms.json" \
      --d2bd-uid "$(id -u)" \
      --d2bd-gid "$(id -g)" \
      --test-mode
  ) >"$scratch/broker/serve.log" 2>&1 &
  broker_pid=$!
  add_cleanup "kill $broker_pid >/dev/null 2>&1 || true"
  wait_for_socket "$socket_path"
  python3 - "$socket_path" "$(id -u)" <<'PY'
import json
import socket
import struct
import sys
import time

socket_path = sys.argv[1]
peer_uid = int(sys.argv[2])
samples = []
for _ in range(100):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
    s.connect(socket_path)
    payload = json.dumps({
        "request": {"kind": "ValidateBundle"},
        "testPeerUid": peer_uid,
    }).encode()
    frame = struct.pack("<I", len(payload)) + payload
    start = time.perf_counter_ns()
    s.sendall(frame)
    response = s.recv(1024 * 1024)
    end = time.perf_counter_ns()
    s.close()
    if len(response) < 4:
        raise SystemExit("broker response shorter than length prefix")
    declared = struct.unpack("<I", response[:4])[0]
    body = response[4:]
    if declared != len(body):
        raise SystemExit("broker response length prefix mismatch")
    parsed = json.loads(body.decode())
    kind = parsed.get("kind")
    if kind not in {"ValidateBundleOk", "ValidateBundle"}:
        raise SystemExit(f"unexpected broker response kind: {kind!r}")
    samples.append((end - start + 999_999) // 1_000_000)

samples.sort()
index = max(0, min(len(samples) - 1, ((99 * len(samples) + 99) // 100) - 1))
print(samples[index])
PY
  kill "$broker_pid"
  wait "$broker_pid" || true
}

measure_vm_start_dry_run_ms() {
  local sample1 sample2 sample3
  measure_once() {
    local start_ms end_ms
    start_ms=$(d2b_now_ms)
    D2B_MANIFEST_PATH="$bundle_root/vms.json" \
    D2B_BUNDLE_PATH="$bundle_root/bundle.json" \
      "$cli_bin" vm start corp-vm --dry-run --json >/dev/null
    end_ms=$(d2b_now_ms)
    printf '%s\n' $((end_ms - start_ms))
  }

  sample1=$(measure_once)
  sample2=$(measure_once)
  sample3=$(measure_once)
  median_of_three "$sample1" "$sample2" "$sample3"
}

assert_budget() {
  local label="$1" actual_ms="$2" budget_ms="$3"
  local allowed_ms
  allowed_ms=$(( (budget_ms * (100 + MARGIN_PERCENT) + 99) / 100 ))
  if [ "$actual_ms" -le "$allowed_ms" ]; then
    ok "$label: ${actual_ms}ms <= ${allowed_ms}ms threshold (budget ${budget_ms}ms +${MARGIN_PERCENT}%)"
  else
    fail "$label regression: ${actual_ms}ms exceeds ${allowed_ms}ms threshold (budget ${budget_ms}ms +${MARGIN_PERCENT}%)"
    exit 1
  fi
}

daemon_hello_ms=$(measure_daemon_cold_start_ms)
broker_validate_p99_ms=$(measure_broker_validate_p99_ms)
vm_start_dry_run_ms=$(measure_vm_start_dry_run_ms)
identity_output=$(measure_identity_derivation)
mapfile -t identity_samples <<<"$identity_output"
if [ "${#identity_samples[@]}" -ne 3 ]; then
  fail "identity derivation benchmark did not return three measured samples"
  exit 1
fi
identity_sample1_ms=${identity_samples[0]%% *}
identity_sample2_ms=${identity_samples[1]%% *}
identity_sample3_ms=${identity_samples[2]%% *}
identity_derivation_ms=$(median_of_three \
  "$identity_sample1_ms" "$identity_sample2_ms" "$identity_sample3_ms")

assert_budget "derive and collision-check 4,096 canonical IDs" \
  "$identity_derivation_ms" "$IDENTITY_DERIVATION_BUDGET_MS"
for sample in "${identity_samples[@]}"; do
  sample_rss_kib=${sample##* }
  if [ "$sample_rss_kib" -gt "$IDENTITY_DERIVATION_RSS_KIB" ]; then
    fail "identity derivation peak RSS ${sample_rss_kib}KiB exceeds ${IDENTITY_DERIVATION_RSS_KIB}KiB ceiling"
    exit 1
  fi
done
ok "identity derivation peak RSS stayed within ${IDENTITY_DERIVATION_RSS_KIB}KiB"
assert_budget "daemon cold start → first Hello" "$daemon_hello_ms" "$DAEMON_HELLO_BUDGET_MS"
assert_budget "broker ValidateBundle p99" "$broker_validate_p99_ms" "$BROKER_VALIDATE_P99_BUDGET_MS"
assert_budget "d2b vm start --dry-run wall time" "$vm_start_dry_run_ms" "$VM_START_DRY_RUN_BUDGET_MS"
