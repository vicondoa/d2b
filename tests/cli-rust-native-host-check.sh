#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=cli-rust-native-common.sh
. "$HERE/cli-rust-native-common.sh"

log "==> tests/cli-rust-native-host-check.sh"
scratch=$(nl_mktemp .cli-rust-native-host-check.XXXXXX)

cli=$(nl_cli_native_bin)
daemon=$(nl_daemon_native_bin)
bundle_root=$(nl_cli_smoke_bundle_tree)
runner_drift_root=$(nl_cli_smoke_bundle_tree_runner_drift)
pass_fixture="$scratch/host-pass.json"
warn_fixture="$scratch/host-warn.json"
fail_fixture="$scratch/host-fail-fixture.json"
builtin_fixture="$scratch/host-builtin.fixture.json"
modules_error_fixture="$scratch/host-modules-error.fixture.json"
nft_error_fixture="$scratch/host-nft-error.fixture.json"
ufw_error_fixture="$scratch/host-ufw-error.fixture.json"
daemon_fail_fixture="$scratch/host-daemon-fail.fixture.json"
nl_write_host_check_fixture_pass "$pass_fixture" "$bundle_root"
nl_write_host_check_fixture_warn "$warn_fixture" "$bundle_root"
nl_write_host_check_fixture_fail "$fail_fixture" "$bundle_root"

jq '.loadedModules |= map(select(. != "kvm_intel")) | .builtInModules = ["kvm_intel"]' \
  "$pass_fixture" > "$builtin_fixture"
jq '.loadedModules = null | .loadedModulesError = "forced /proc/modules read failure"' \
  "$pass_fixture" > "$modules_error_fixture"
jq '.nftHasNixlingTable = null | .nftError = "forced nft probe failure"' \
  "$pass_fixture" > "$nft_error_fixture"
jq '.ufwActive = null | .ufwError = "forced ufw probe failure"' \
  "$pass_fixture" > "$ufw_error_fixture"
jq '.kernelRelease = "6.5.0-nixling" | .cgroupV2Present = false' \
  "$pass_fixture" > "$daemon_fail_fixture"

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

daemon_host_check_request() {
  local label="$1" bundle_path="$2" fixture="$3" strict_json="$4"
  local bundle_dir run_tag socket_dir socket_path state_lock locks_dir config_json output server_pid
  bundle_dir=$(dirname "$bundle_path")
  case "$label" in
    daemon-pass) run_tag='dp' ;;
    daemon-fail) run_tag='df' ;;
    daemon-runner-strict) run_tag='drs' ;;
    *) run_tag='hc' ;;
  esac
  socket_dir="$scratch/$run_tag"
  socket_path="$socket_dir/public.sock"
  state_lock="$socket_dir/daemon.lock"
  locks_dir="$socket_dir/locks"
  config_json="$scratch/$run_tag-config.json"

  mkdir -p "$socket_dir" "$locks_dir"
  chmod 0755 "$socket_dir"
  cat > "$config_json" <<EOF2
{
  "publicSocketPath": "$socket_path",
  "brokerSocketPath": "$socket_dir/priv.sock",
  "stateLockPath": "$state_lock",
  "locksDir": "$locks_dir",
  "daemonUser": "root",
  "daemonGroup": "root",
  "publicSocketGroup": "$(id -gn)",
  "launcherUsers": ["launcher-user"],
  "adminUsers": ["admin-user"],
  "serverVersion": "0.4.0",
  "acceptedClientVersionRange": ">=0.4.0, <0.5.0",
  "artifacts": {
    "publicManifestPath": "$bundle_dir/vms.json",
    "bundlePath": "$bundle_path",
    "hostPath": "$bundle_dir/host.json",
    "processesPath": "$bundle_dir/processes.json",
    "closuresDir": "$bundle_dir/closures"
  }
}
EOF2

  (
    export NIXLINGD_TEST_PEER_UID=60003
    export NIXLINGD_TEST_PEER_GID=60003
    export NIXLINGD_TEST_PEER_USERNAME=launcher-user
    export NIXLINGD_TEST_PEER_GROUPS=wheel
    export NIXLING_HOST_CHECK_FIXTURE="$fixture"
    "$daemon" serve \
      --config "$config_json" \
      --test-listen-on "$socket_path" \
      --state-lock "$state_lock" \
      --locks-dir "$locks_dir" \
      --once \
      --allow-unprivileged-runtime-dir \
      --no-drop-privileges
  ) > "$scratch/$label.server.log" 2>&1 &
  server_pid=$!
  if ! wait_for_socket "$socket_path"; then
    cat "$scratch/$label.server.log" >&2 || true
    return 1
  fi
  output=$("$daemon" test-client --socket "$socket_path" \
    --frame-json '{"type":"hello","clientVersion":">=0.4.0, <0.5.0","supportedFeatures":[]}' \
    --frame-json "{\"type\":\"hostCheck\",\"strict\":$strict_json}" 2>&1)
  wait "$server_pid"
  printf '%s\n' "$output" | tail -n1
}

set +e
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_HOST_CHECK_FIXTURE="$pass_fixture" \
  "$cli" host check --read-only --human > "$scratch/host.human" 2> "$scratch/host.human.stderr"
rc_human=$?
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_HOST_CHECK_FIXTURE="$pass_fixture" \
  "$cli" host check --read-only --json > "$scratch/host.json" 2> "$scratch/host.json.stderr"
rc_json=$?
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_HOST_CHECK_FIXTURE="$builtin_fixture" \
  "$cli" host check --read-only --json > "$scratch/host-builtin.json" 2> "$scratch/host-builtin.stderr"
rc_builtin=$?
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_HOST_CHECK_FIXTURE="$pass_fixture" \
NIXLING_TEST_UFW_ACTIVE=1 \
  "$cli" host check --read-only --json > "$scratch/host-ufw-warn.json" 2> "$scratch/host-ufw-warn.stderr"
rc_ufw_warn=$?
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_HOST_CHECK_FIXTURE="$pass_fixture" \
NIXLING_TEST_SYSTEMCTL_UNAVAILABLE=1 \
  "$cli" host check --read-only --json > "$scratch/host-systemctl-unavailable.json" 2> "$scratch/host-systemctl-unavailable.stderr"
rc_systemctl_unavailable=$?
NIXLING_BUNDLE_PATH="$runner_drift_root/bundle.json" \
NIXLING_HOST_CHECK_FIXTURE="$pass_fixture" \
  "$cli" host check --read-only --json > "$scratch/host-runner-warn.json" 2> "$scratch/host-runner-warn.stderr"
rc_runner_warn=$?
NIXLING_BUNDLE_PATH="$runner_drift_root/bundle.json" \
NIXLING_HOST_CHECK_FIXTURE="$pass_fixture" \
  "$cli" host check --read-only --strict --json > "$scratch/host-runner-strict.json" 2> "$scratch/host-runner-strict.stderr"
rc_runner_strict=$?
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_HOST_CHECK_FIXTURE="$fail_fixture" \
  "$cli" host check --read-only --json > "$scratch/host-fail.json" 2> "$scratch/host-fail.stderr"
rc_fail=$?
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_HOST_CHECK_FIXTURE="$modules_error_fixture" \
  "$cli" host check --read-only --json > "$scratch/host-modules-error.json" 2> "$scratch/host-modules-error.stderr"
rc_modules_error=$?
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_HOST_CHECK_FIXTURE="$nft_error_fixture" \
  "$cli" host check --read-only --json > "$scratch/host-nft-error.json" 2> "$scratch/host-nft-error.stderr"
rc_nft_error=$?
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_HOST_CHECK_FIXTURE="$ufw_error_fixture" \
  "$cli" host check --read-only --json > "$scratch/host-ufw-error.json" 2> "$scratch/host-ufw-error.stderr"
rc_ufw_error=$?
"$cli" host check --bogus > /dev/null 2> "$scratch/host-usage.stderr"
rc_usage=$?
set -e

[ "$rc_human" -eq 0 ] || { fail "host check --human should exit 0 for the pass fixture"; exit 1; }
[ "$rc_json" -eq 0 ] || { fail "host check --json should exit 0 for the pass fixture"; exit 1; }
[ "$rc_builtin" -eq 0 ] || { fail "host check should accept built-in kernel modules"; exit 1; }
[ "$rc_ufw_warn" -eq 1 ] || { fail "host check should warn when ufw.service is active"; exit 1; }
[ "$rc_systemctl_unavailable" -eq 1 ] || { fail "host check should warn when systemctl is unavailable"; exit 1; }
[ "$rc_runner_warn" -eq 1 ] || { fail "host check runner-parity warning case should exit 1"; exit 1; }
[ "$rc_runner_strict" -eq 2 ] || { fail "host check --strict runner-parity case should exit 2"; exit 1; }
[ "$rc_fail" -eq 2 ] || { fail "host check hard-failure case should exit 2"; exit 1; }
[ "$rc_modules_error" -eq 1 ] || { fail "host check should surface /proc/modules probe failures as internal errors"; exit 1; }
[ "$rc_nft_error" -eq 1 ] || { fail "host check should surface nft probe failures as internal errors"; exit 1; }
[ "$rc_ufw_error" -eq 1 ] || { fail "host check should surface ufw probe failures as internal errors"; exit 1; }
[ "$rc_usage" -eq 3 ] || { fail "host check usage errors should exit 3"; exit 1; }
ok "host check exit codes stay distinct across pass/warn/fail/internal-error/usage cases"

nl_assert_json_schema "$ROOT/docs/reference/cli-output/host-check.schema.json" "$scratch/host.json"
ok "host check --json validates against docs/reference/cli-output/host-check.schema.json"

if jq -e '.exitCode == 0 and .summary.fail == 0 and .summary.warn == 0' "$scratch/host.json" >/dev/null 2>&1; then
  ok "host check pass fixture renders a green summary"
else
  fail "host check pass fixture output regressed"
  exit 1
fi

if jq -e '.exitCode == 0 and any(.findings[]; .id == "kernel-module:kvm_intel" and .severity == "pass" and (.message | contains("built into the running kernel")))' "$scratch/host-builtin.json" >/dev/null 2>&1; then
  ok "host check treats built-in kernel modules as present"
else
  fail "built-in kernel module detection regressed"
  exit 1
fi

if jq -e '.exitCode == 1 and any(.findings[]; .id == "firewall-coexistence" and .severity == "warn" and .message == "firewalld_active=false ufw_active=true")' "$scratch/host-ufw-warn.json" >/dev/null 2>&1; then
  ok "host check warns when ufw.service is active"
else
  fail "ufw active warning case regressed"
  exit 1
fi

if jq -e '.exitCode == 1 and any(.findings[]; .id == "firewall-coexistence" and .severity == "warn" and (.message | contains("could not be fully determined")) and .detail == "systemctl probe unavailable on this host")' "$scratch/host-systemctl-unavailable.json" >/dev/null 2>&1; then
  ok "host check warns when systemctl is unavailable instead of claiming inactive firewalls"
else
  fail "systemctl-unavailable warning case regressed"
  exit 1
fi

if jq -e '.exitCode == 1 and .summary.warn > 0 and .summary.fail == 0 and any(.findings[]; .id == "runner-parity" and .severity == "warn")' "$scratch/host-runner-warn.json" >/dev/null 2>&1; then
  ok "runner parity drift is advisory without --strict"
else
  fail "host check warning case regressed"
  exit 1
fi

if jq -e '.exitCode == 2 and .summary.fail > 0 and any(.findings[]; .id == "runner-parity" and .severity == "fail")' "$scratch/host-runner-strict.json" >/dev/null 2>&1; then
  ok "runner parity drift becomes fatal under --strict"
else
  fail "host check strict case regressed"
  exit 1
fi

if jq -e '.kind == "internal-io" and .owningCommand == "host check" and .code == 50 and (.message | contains("forced /proc/modules read failure"))' "$scratch/host-modules-error.stderr" >/dev/null 2>&1; then
  ok "host check wraps /proc/modules probe failures in the operator error envelope"
else
  fail "/proc/modules probe error envelope regressed"
  exit 1
fi

if jq -e '.kind == "internal-io" and .owningCommand == "host check" and .code == 50 and (.message | contains("forced nft probe failure"))' "$scratch/host-nft-error.stderr" >/dev/null 2>&1; then
  ok "host check wraps nft probe failures in the operator error envelope"
else
  fail "nft probe error envelope regressed"
  exit 1
fi

if jq -e '.kind == "internal-io" and .owningCommand == "host check" and .code == 50 and (.message | contains("forced ufw probe failure"))' "$scratch/host-ufw-error.stderr" >/dev/null 2>&1; then
  ok "host check wraps ufw probe failures in the operator error envelope"
else
  fail "ufw probe error envelope regressed"
  exit 1
fi

if grep -Fq 'summary: pass=' "$scratch/host.human" && grep -Fq 'PASS' "$scratch/host.human"; then
  ok "host check --human groups findings by severity"
else
  fail "host check --human output regressed"
  exit 1
fi

daemon_host_check_request daemon-pass "$bundle_root/bundle.json" "$pass_fixture" false > "$scratch/daemon-pass.json"
daemon_host_check_request daemon-fail "$bundle_root/bundle.json" "$daemon_fail_fixture" false > "$scratch/daemon-fail.json"
daemon_host_check_request daemon-runner-strict "$runner_drift_root/bundle.json" "$pass_fixture" true > "$scratch/daemon-runner-strict.json"

if jq -e '.summary.failures == 0 and .summary.warnings == 0 and any(.checks[]; .name == "kernel-module:kvm") and any(.checks[]; .name == "nftables-table") and any(.checks[]; .name == "firewall-coexistence") and any(.checks[]; .name == "runner-parity")' "$scratch/daemon-pass.json" >/dev/null 2>&1; then
  ok "daemon host-check path runs the full W2 probe battery"
else
  fail "daemon host-check pass case regressed"
  exit 1
fi

if jq -e '.summary.failures >= 2 and any(.checks[]; .name == "kernel-version" and .status == "fail") and any(.checks[]; .name == "cgroup-v2" and .status == "fail")' "$scratch/daemon-fail.json" >/dev/null 2>&1; then
  ok "daemon host-check fails closed on old kernels and missing cgroup-v2"
else
  fail "daemon host-check failure semantics regressed"
  exit 1
fi

if jq -e '.summary.failures > 0 and any(.checks[]; .name == "runner-parity" and .status == "fail")' "$scratch/daemon-runner-strict.json" >/dev/null 2>&1; then
  ok "daemon host-check surfaces strict runner-parity failures"
else
  fail "daemon strict runner-parity case regressed"
  exit 1
fi

log "==> cli-rust-native-host-check OK"
