#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=cli-rust-native-common.sh
. "$HERE/cli-rust-native-common.sh"

log "==> tests/cli-rust-native-list.sh"
scratch=$(nl_mktemp .cli-rust-native-list.XXXXXX)

bundle_root=$(nl_cli_smoke_bundle_tree)
system_fixture="$scratch/system-state.json"
nl_write_system_state_fixture "$system_fixture"
cli=$(nl_cli_native_bin)

NIXLING_MANIFEST_PATH="$bundle_root/vms.json" \
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_TEST_SYSTEM_STATE_JSON="$system_fixture" \
  "$cli" list --human > "$scratch/list.human"
NIXLING_MANIFEST_PATH="$bundle_root/vms.json" \
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
NIXLING_TEST_SYSTEM_STATE_JSON="$system_fixture" \
  "$cli" list --json > "$scratch/list.json"

nl_assert_json_schema "$ROOT/docs/reference/cli-output/list.schema.json" "$scratch/list.json"
ok "list --json validates against docs/reference/cli-output/list.schema.json"

if jq -e '
  type == "array"
  and length == 2
  and any(.[]; .name == "corp-vm" and .env == "work" and .status == "stopped" and .runnerParityOk == true)
  and any(.[]; .name == "sys-work-net" and .isNetVm == true and .status == "running" and .runnerParityOk == true)
' "$scratch/list.json" >/dev/null 2>&1; then
  ok "list --json returns expected VM inventory"
else
  fail "list --json did not match the expected smoke inventory"
  exit 1
fi

if grep -Fq 'corp-vm' "$scratch/list.human" && grep -Fq 'systemd (net-vm)' "$scratch/list.human"; then
  ok "list --human renders the expected table"
else
  fail "list --human output regressed"
  exit 1
fi

log "==> cli-rust-native-list OK"
