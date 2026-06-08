#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=cli-rust-native-common.sh
. "$HERE/cli-rust-native-common.sh"

log "==> tests/cli-rust-native-status.sh"
scratch=$(nl_mktemp .cli-rust-native-status.XXXXXX)

bundle_root=$(nl_cli_smoke_bundle_tree)
system_fixture="$scratch/system-state.json"
nl_write_system_state_fixture "$system_fixture"
cli=$(nl_cli_native_bin)

NIXLING_MANIFEST_PATH="$bundle_root/vms.json" \
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_TEST_SYSTEM_STATE_JSON="$system_fixture" \
  "$cli" status --vm corp-vm --human > "$scratch/status.human"
NIXLING_MANIFEST_PATH="$bundle_root/vms.json" \
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_TEST_SYSTEM_STATE_JSON="$system_fixture" \
  "$cli" status --vm corp-vm --json > "$scratch/status.json"
NIXLING_MANIFEST_PATH="$bundle_root/vms.json" \
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_TEST_SYSTEM_STATE_JSON="$system_fixture" \
  "$cli" status corp-vm --json > "$scratch/status-positional.json"
NIXLING_MANIFEST_PATH="$bundle_root/vms.json" \
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_TEST_SYSTEM_STATE_JSON="$system_fixture" \
  "$cli" status --check-bridges --json > "$scratch/status-bridges.json"

cmp -s "$scratch/status.json" "$scratch/status-positional.json"
ok "status --vm <name> and status <name> stay equivalent"

nl_assert_json_schema "$ROOT/docs/reference/cli-output/status.schema.json" "$scratch/status.json"
ok "status --json validates against docs/reference/cli-output/status.schema.json"

if jq -e '
  .name == "corp-vm"
  and .env == "work"
  and .services.nixling == "inactive"
  and .services.microvm == "inactive"
  and .services.virtiofsd == "inactive"
  and .pendingRestart == false
  and .runtime == "unknown (daemon-experimental, W4 not landed)"
  and (.declaredRoles | index("cloud-hypervisor-runner") != null)
  and .runnerParity.runnerParityOk == true
' "$scratch/status.json" >/dev/null 2>&1; then
  ok "status --json exposes declared/static W2 state"
else
  fail "status --json output regressed"
  exit 1
fi

if jq -e '.status == "not-yet-implemented" and .mode == "check-bridges"' "$scratch/status-bridges.json" >/dev/null 2>&1; then
  ok "status --check-bridges returns the frozen not-yet-implemented envelope"
else
  fail "status --check-bridges output regressed"
  exit 1
fi

if grep -Fq 'runner parity: ok' "$scratch/status.human" && grep -Fq 'Bridge health' "$scratch/status.human"; then
  ok "status --human renders runtime, parity, and bridge sections"
else
  fail "status --human output regressed"
  exit 1
fi

log "==> cli-rust-native-status OK"
