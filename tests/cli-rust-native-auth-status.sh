#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=cli-rust-native-common.sh
. "$HERE/cli-rust-native-common.sh"

log "==> tests/cli-rust-native-auth-status.sh"
scratch=$(nl_mktemp .cli-rust-native-auth-status.XXXXXX)

cli=$(nl_cli_native_bin)
launcher_fixture="$scratch/auth-launcher.json"
none_fixture="$scratch/auth-none.json"
admin_fixture="$scratch/auth-admin.json"
nl_write_auth_status_fixture "$launcher_fixture" launcher
nl_write_auth_status_fixture "$none_fixture" none
nl_write_auth_status_fixture "$admin_fixture" admin

NIXLING_AUTH_STATUS_FIXTURE="$launcher_fixture" \
NIXLING_TEST_LAUNCHER_UIDS=1000 \
  "$cli" auth status --test-uid 1000 --human > "$scratch/auth.human"
NIXLING_AUTH_STATUS_FIXTURE="$launcher_fixture" \
NIXLING_TEST_LAUNCHER_UIDS=1000 \
  "$cli" auth status --test-uid 1000 --json > "$scratch/auth.json"
NIXLING_AUTH_STATUS_FIXTURE="$none_fixture" \
  "$cli" auth status --test-uid 2000 --json > "$scratch/auth-none.json.out"
NIXLING_AUTH_STATUS_FIXTURE="$admin_fixture" \
NIXLING_TEST_ADMIN_UIDS=2001 \
  "$cli" auth status --test-uid 2001 --json > "$scratch/auth-admin.json.out"

nl_assert_json_schema "$ROOT/docs/reference/cli-output/auth-status.schema.json" "$scratch/auth.json"
ok "auth status --json validates against docs/reference/cli-output/auth-status.schema.json"

if jq -e '.role == "launcher" and .effectiveUid == 1000 and (.allowedSubcommands | index("up") != null) and (.deniedSubcommands | map(.name) | index("audit") != null)' "$scratch/auth.json" >/dev/null 2>&1; then
  ok "launcher role exposes launcher-allowed subcommands and keeps audit denied"
else
  fail "auth status launcher case regressed"
  exit 1
fi

if jq -e '.role == "none" and (.allowedSubcommands | sort == ["auth status","host check","list","status"])' "$scratch/auth-none.json.out" >/dev/null 2>&1; then
  ok "none role stays read-only"
else
  fail "auth status none-role case regressed"
  exit 1
fi

if jq -e '.role == "admin" and (.deniedSubcommands | length) == 0 and (.allowedSubcommands | index("audit") != null)' "$scratch/auth-admin.json.out" >/dev/null 2>&1; then
  ok "admin role gains audit access"
else
  fail "auth status admin-role case regressed"
  exit 1
fi

if grep -Fq 'role: launcher' "$scratch/auth.human" && grep -Fq 'audit:' "$scratch/auth.human"; then
  ok "auth status --human summarizes role and denied audit access"
else
  fail "auth status --human output regressed"
  exit 1
fi

log "==> cli-rust-native-auth-status OK"
