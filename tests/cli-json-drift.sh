#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=cli-rust-native-common.sh
. "$HERE/cli-rust-native-common.sh"

log "==> tests/cli-json-drift.sh"
xtask_bin=$(nl_cargo_bin_path workspace xtask)
if [ ! -x "$xtask_bin" ]; then
  nl_cli_toolchain_shell "cd '$ROOT/packages' && CARGO_TARGET_DIR='$(nl_cargo_target_dir workspace)' cargo build -q --manifest-path '$ROOT/packages/Cargo.toml' -p xtask --bin xtask"
fi
(cd "$ROOT/packages" && "$xtask_bin" gen-cli-schemas) >/dev/null
if git --no-pager diff --exit-code -- docs/reference/cli-output/*.schema.json >/dev/null; then
  ok "cli-output schemas match cargo xtask gen-cli-schemas"
else
  git --no-pager diff -- docs/reference/cli-output/*.schema.json | head -120 >&2 || true
  fail "cli-output schema drift after cargo xtask gen-cli-schemas"
fi

scratch=$(nl_mktemp .cli-json-drift.XXXXXX)

bundle_root=$(nl_cli_smoke_bundle_tree)
system_fixture="$scratch/system-state.json"
host_fixture="$scratch/host-pass.json"
auth_fixture="$scratch/auth-launcher.json"
nl_write_system_state_fixture "$system_fixture"
nl_write_host_check_fixture_pass "$host_fixture" "$bundle_root"
nl_write_auth_status_fixture "$auth_fixture" launcher
cli=$(nl_cli_native_bin)
legacy=$(nl_legacy_cli_bin)

run_native() {
  local out="$1"; shift
  "$@" > "$out"
}

run_bundle_native() {
  local out="$1"; shift
  NIXLING_MANIFEST_PATH="$bundle_root/vms.json" \
  NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
    run_native "$out" "$cli" "$@"
}

run_bundle_native_with_shape() {
  local shape="$1" out="$2"
  shift 2
  NIXLING_MANIFEST_PATH="$bundle_root/vms.json" \
  NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
  NIXLING_TEST_DEPLOYMENT_SHAPE="$shape" \
    run_native "$out" "$cli" "$@"
}

compare_with_golden() {
  local actual="$1" golden="$2" expected
  expected="$scratch/$(basename "$golden").expected"
  sed '/^#/d' "$golden" > "$expected"
  if cmp -s "$actual" "$expected"; then
    ok "golden match: $(basename "$golden")"
  else
    diff -u "$expected" "$actual" >&2 || true
    fail "golden drift: $(basename "$golden")"
    exit 1
  fi
}


NIXLING_MANIFEST_PATH="$bundle_root/vms.json" \
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_TEST_SYSTEM_STATE_JSON="$system_fixture" \
  run_native "$scratch/list-human.out" "$cli" list --human
NIXLING_MANIFEST_PATH="$bundle_root/vms.json" \
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_TEST_SYSTEM_STATE_JSON="$system_fixture" \
  run_native "$scratch/list-json.out" "$cli" list --json
NIXLING_MANIFEST_PATH="$bundle_root/vms.json" \
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_TEST_SYSTEM_STATE_JSON="$system_fixture" \
  run_native "$scratch/status-human.out" "$cli" status --vm corp-vm --human
NIXLING_MANIFEST_PATH="$bundle_root/vms.json" \
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_TEST_SYSTEM_STATE_JSON="$system_fixture" \
  run_native "$scratch/status-json.out" "$cli" status --vm corp-vm --json
NIXLING_LEGACY_CLI="$legacy" \
NIXLING_PUBLIC_SOCKET="$scratch/missing.sock" \
NIXLING_AUDIT_TESTMODE_KVM_MODE=660 \
  run_native "$scratch/audit-human.out" "$cli" audit --human 2> "$scratch/audit-human.stderr"
NIXLING_LEGACY_CLI="$legacy" \
NIXLING_PUBLIC_SOCKET="$scratch/missing.sock" \
NIXLING_AUDIT_TESTMODE_KVM_MODE=660 \
  run_native "$scratch/audit-json.out" "$cli" audit --json 2> "$scratch/audit-json.stderr"
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_HOST_CHECK_FIXTURE="$host_fixture" \
  run_native "$scratch/host-check-human.out" "$cli" host check --read-only --human
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_HOST_CHECK_FIXTURE="$host_fixture" \
  run_native "$scratch/host-check-json.out" "$cli" host check --read-only --json
NIXLING_AUTH_STATUS_FIXTURE="$auth_fixture" \
NIXLING_TEST_LAUNCHER_UIDS=1000 \
  run_native "$scratch/auth-status-human.out" "$cli" auth status --test-uid 1000 --human
NIXLING_AUTH_STATUS_FIXTURE="$auth_fixture" \
NIXLING_TEST_LAUNCHER_UIDS=1000 \
  run_native "$scratch/auth-status-json.out" "$cli" auth status --test-uid 1000 --json

run_bundle_native "$scratch/vm-start-dry-run-human.out" vm start corp-vm --dry-run --human
run_bundle_native "$scratch/vm-start-dry-run-json.out" vm start corp-vm --dry-run --json
run_bundle_native "$scratch/vm-stop-dry-run-human.out" vm stop corp-vm --dry-run --human
run_bundle_native "$scratch/vm-stop-dry-run-json.out" vm stop corp-vm --dry-run --json
run_bundle_native "$scratch/vm-restart-dry-run-human.out" vm restart corp-vm --dry-run --human
run_bundle_native "$scratch/vm-restart-dry-run-json.out" vm restart corp-vm --dry-run --json
run_bundle_native_with_shape all-daemon "$scratch/host-prepare-dry-run-human.out" host prepare --dry-run --human
run_bundle_native_with_shape all-daemon "$scratch/host-prepare-dry-run-json.out" host prepare --dry-run --json
run_bundle_native_with_shape all-daemon "$scratch/host-destroy-dry-run-human.out" host destroy --dry-run --human
run_bundle_native_with_shape all-daemon "$scratch/host-destroy-dry-run-json.out" host destroy --dry-run --json
run_bundle_native "$scratch/switch-dry-run-human.out" switch corp-vm --dry-run --human
run_bundle_native "$scratch/switch-dry-run-json.out" switch corp-vm --dry-run --json
run_bundle_native "$scratch/boot-dry-run-human.out" boot corp-vm --dry-run --human
run_bundle_native "$scratch/boot-dry-run-json.out" boot corp-vm --dry-run --json
run_bundle_native "$scratch/test-dry-run-human.out" test corp-vm --dry-run --human
run_bundle_native "$scratch/test-dry-run-json.out" test corp-vm --dry-run --json
run_bundle_native "$scratch/rollback-dry-run-human.out" rollback corp-vm --dry-run --human
run_bundle_native "$scratch/rollback-dry-run-json.out" rollback corp-vm --dry-run --json
run_bundle_native "$scratch/gc-dry-run-human.out" gc --dry-run --human
run_bundle_native "$scratch/gc-dry-run-json.out" gc --dry-run --json
run_bundle_native "$scratch/keys-rotate-dry-run-human.out" keys rotate corp-vm --dry-run --human
run_bundle_native "$scratch/keys-rotate-dry-run-json.out" keys rotate corp-vm --dry-run --json
run_bundle_native "$scratch/trust-dry-run-human.out" trust corp-vm --dry-run --human
run_bundle_native "$scratch/trust-dry-run-json.out" trust corp-vm --dry-run --json
run_bundle_native "$scratch/rotate-known-host-dry-run-human.out" rotate-known-host corp-vm --dry-run --human
run_bundle_native "$scratch/rotate-known-host-dry-run-json.out" rotate-known-host corp-vm --dry-run --json
run_bundle_native_with_shape all-daemon "$scratch/migrate-dry-run-human.out" migrate --dry-run --human
run_bundle_native_with_shape all-daemon "$scratch/migrate-dry-run-json.out" migrate --dry-run --json
run_native "$scratch/host-install-dry-run-human.out" "$cli" host install --dry-run --human
run_native "$scratch/host-install-dry-run-json.out" "$cli" host install --dry-run --json
run_bundle_native "$scratch/usb-attach-dry-run-human.out" usb attach corp-vm 1-2 --dry-run --human
run_bundle_native "$scratch/usb-attach-dry-run-json.out" usb attach corp-vm 1-2 --dry-run --json
run_bundle_native "$scratch/usb-detach-dry-run-human.out" usb detach corp-vm 1-2 --dry-run --human
run_bundle_native "$scratch/usb-detach-dry-run-json.out" usb detach corp-vm 1-2 --dry-run --json

compare_with_golden "$scratch/list-human.out" "$ROOT/tests/golden/cli-output/list-human.golden"
compare_with_golden "$scratch/list-json.out" "$ROOT/tests/golden/cli-output/list-json.golden"
compare_with_golden "$scratch/status-human.out" "$ROOT/tests/golden/cli-output/status-human.golden"
compare_with_golden "$scratch/status-json.out" "$ROOT/tests/golden/cli-output/status-json.golden"
compare_with_golden "$scratch/audit-human.out" "$ROOT/tests/golden/cli-output/audit-human.golden"
compare_with_golden "$scratch/audit-json.out" "$ROOT/tests/golden/cli-output/audit-json.golden"
compare_with_golden "$scratch/host-check-human.out" "$ROOT/tests/golden/cli-output/host-check-human.golden"
compare_with_golden "$scratch/host-check-json.out" "$ROOT/tests/golden/cli-output/host-check-json.golden"
compare_with_golden "$scratch/auth-status-human.out" "$ROOT/tests/golden/cli-output/auth-status-human.golden"
compare_with_golden "$scratch/auth-status-json.out" "$ROOT/tests/golden/cli-output/auth-status-json.golden"
compare_with_golden "$scratch/vm-start-dry-run-human.out" "$ROOT/tests/golden/cli-output/vm-start-dry-run-human.golden"
compare_with_golden "$scratch/vm-start-dry-run-json.out" "$ROOT/tests/golden/cli-output/vm-start-dry-run-json.golden"
compare_with_golden "$scratch/vm-stop-dry-run-human.out" "$ROOT/tests/golden/cli-output/vm-stop-dry-run-human.golden"
compare_with_golden "$scratch/vm-stop-dry-run-json.out" "$ROOT/tests/golden/cli-output/vm-stop-dry-run-json.golden"
compare_with_golden "$scratch/vm-restart-dry-run-human.out" "$ROOT/tests/golden/cli-output/vm-restart-dry-run-human.golden"
compare_with_golden "$scratch/vm-restart-dry-run-json.out" "$ROOT/tests/golden/cli-output/vm-restart-dry-run-json.golden"
compare_with_golden "$scratch/host-prepare-dry-run-human.out" "$ROOT/tests/golden/cli-output/host-prepare-dry-run-human.golden"
compare_with_golden "$scratch/host-prepare-dry-run-json.out" "$ROOT/tests/golden/cli-output/host-prepare-dry-run-json.golden"
compare_with_golden "$scratch/host-destroy-dry-run-human.out" "$ROOT/tests/golden/cli-output/host-destroy-dry-run-human.golden"
compare_with_golden "$scratch/host-destroy-dry-run-json.out" "$ROOT/tests/golden/cli-output/host-destroy-dry-run-json.golden"
compare_with_golden "$scratch/switch-dry-run-human.out" "$ROOT/tests/golden/cli-output/switch-dry-run-human.golden"
compare_with_golden "$scratch/switch-dry-run-json.out" "$ROOT/tests/golden/cli-output/switch-dry-run-json.golden"
compare_with_golden "$scratch/boot-dry-run-human.out" "$ROOT/tests/golden/cli-output/boot-dry-run-human.golden"
compare_with_golden "$scratch/boot-dry-run-json.out" "$ROOT/tests/golden/cli-output/boot-dry-run-json.golden"
compare_with_golden "$scratch/test-dry-run-human.out" "$ROOT/tests/golden/cli-output/test-dry-run-human.golden"
compare_with_golden "$scratch/test-dry-run-json.out" "$ROOT/tests/golden/cli-output/test-dry-run-json.golden"
compare_with_golden "$scratch/rollback-dry-run-human.out" "$ROOT/tests/golden/cli-output/rollback-dry-run-human.golden"
compare_with_golden "$scratch/rollback-dry-run-json.out" "$ROOT/tests/golden/cli-output/rollback-dry-run-json.golden"
compare_with_golden "$scratch/gc-dry-run-human.out" "$ROOT/tests/golden/cli-output/gc-dry-run-human.golden"
compare_with_golden "$scratch/gc-dry-run-json.out" "$ROOT/tests/golden/cli-output/gc-dry-run-json.golden"
compare_with_golden "$scratch/keys-rotate-dry-run-human.out" "$ROOT/tests/golden/cli-output/keys-rotate-dry-run-human.golden"
compare_with_golden "$scratch/keys-rotate-dry-run-json.out" "$ROOT/tests/golden/cli-output/keys-rotate-dry-run-json.golden"
compare_with_golden "$scratch/trust-dry-run-human.out" "$ROOT/tests/golden/cli-output/trust-dry-run-human.golden"
compare_with_golden "$scratch/trust-dry-run-json.out" "$ROOT/tests/golden/cli-output/trust-dry-run-json.golden"
compare_with_golden "$scratch/rotate-known-host-dry-run-human.out" "$ROOT/tests/golden/cli-output/rotate-known-host-dry-run-human.golden"
compare_with_golden "$scratch/rotate-known-host-dry-run-json.out" "$ROOT/tests/golden/cli-output/rotate-known-host-dry-run-json.golden"
compare_with_golden "$scratch/migrate-dry-run-human.out" "$ROOT/tests/golden/cli-output/migrate-dry-run-human.golden"
compare_with_golden "$scratch/migrate-dry-run-json.out" "$ROOT/tests/golden/cli-output/migrate-dry-run-json.golden"
compare_with_golden "$scratch/host-install-dry-run-human.out" "$ROOT/tests/golden/cli-output/host-install-dry-run-human.golden"
compare_with_golden "$scratch/host-install-dry-run-json.out" "$ROOT/tests/golden/cli-output/host-install-dry-run-json.golden"
compare_with_golden "$scratch/usb-attach-dry-run-human.out" "$ROOT/tests/golden/cli-output/usb-attach-dry-run-human.golden"
compare_with_golden "$scratch/usb-attach-dry-run-json.out" "$ROOT/tests/golden/cli-output/usb-attach-dry-run-json.golden"
compare_with_golden "$scratch/usb-detach-dry-run-human.out" "$ROOT/tests/golden/cli-output/usb-detach-dry-run-human.golden"
compare_with_golden "$scratch/usb-detach-dry-run-json.out" "$ROOT/tests/golden/cli-output/usb-detach-dry-run-json.golden"

HOME="$scratch/home" XDG_RUNTIME_DIR="$scratch/runtime" "$legacy" list --json > "$scratch/list-v04bash.out"
HOME="$scratch/home" XDG_RUNTIME_DIR="$scratch/runtime" "$legacy" status corp-vm --json > "$scratch/status-v04bash.out"
HOME="$scratch/home" XDG_RUNTIME_DIR="$scratch/runtime" NIXLING_AUDIT_TESTMODE_KVM_MODE=660 "$legacy" audit --json > "$scratch/audit-v04bash.out"

compare_with_golden "$scratch/list-v04bash.out" "$ROOT/tests/golden/cli-output/list.v04bash.golden"
compare_with_golden "$scratch/status-v04bash.out" "$ROOT/tests/golden/cli-output/status.v04bash.golden"
compare_with_golden "$scratch/audit-v04bash.out" "$ROOT/tests/golden/cli-output/audit.v04bash.golden"

jq -S 'map({name,env,graphics,tpm,usbip,staticIp,status,isNetVm})' "$scratch/list-json.out" > "$scratch/list-rust-subset.json"
jq -S '.' "$scratch/list-v04bash.out" > "$scratch/list-bash-subset.json"
cmp -s "$scratch/list-rust-subset.json" "$scratch/list-bash-subset.json"
ok "list rust JSON stays equivalent to the v0.4.0 bash subset"

jq -S '{name,services,current,booted,pendingRestart}' "$scratch/status-json.out" > "$scratch/status-rust-subset.json"
jq -S '.' "$scratch/status-v04bash.out" > "$scratch/status-bash-subset.json"
cmp -s "$scratch/status-rust-subset.json" "$scratch/status-bash-subset.json"
ok "status rust JSON stays equivalent to the v0.4.0 bash subset"

cmp -s "$scratch/audit-json.out" "$scratch/audit-v04bash.out"
ok "audit rust JSON stays identical to the v0.4.0 bash fallback output"

log "==> cli-json-drift OK"
