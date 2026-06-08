#!/usr/bin/env bash
# Layer-1 legacy CLI dispatch parity gate.
#
# Dispatchable in Layer-1 (sandbox-safe, no live host required):
#   - tests/cli-json.sh
#
# Deferred to Layer-2 / live-host coverage:
#   - tests/nixling-store.sh
#   - tests/audio.sh
#   - tests/network-isolation.sh
#   - tests/audit-forwarding.sh
#
# Eval-only tests that inspect the rendered bash wrapper text (for example
# observability-eval.sh) are also out of scope here because they do not
# execute the installed `nixling` command.
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=cli-rust-native-common.sh
. "$HERE/cli-rust-native-common.sh"

log "==> tests/cli-legacy-bash-dispatch.sh"
scratch=$(nl_mktemp .cli-legacy-bash-dispatch.XXXXXX)
add_cleanup "rm -rf -- \"$scratch\""
mkdir -p "$scratch/path"

cli=$(nl_cli_native_bin)
legacy=$(nl_legacy_cli_bin)
real_nix_build=$(command -v nix-build)
shim_cli_out="$scratch/shim-cli-out"
shim_path="$scratch/path"
ln -s "$cli" "$shim_path/nixling"

write_shim_helper="$scratch/write-shim-cli.sh"
cat > "$write_shim_helper" <<'EOF_WRITE_SHIM_HELPER'
#!/usr/bin/env bash
set -euo pipefail

native_cli=$1
legacy_out=$2
shim_cli_out=$3
legacy_cli="$legacy_out/bin/nixling"
manifest_path=$(sed -n 's/^[[:space:]]*MANIFEST=//p' "$legacy_cli" | head -1)
if [ -z "$manifest_path" ]; then
  printf 'cli-legacy-bash-dispatch: could not discover MANIFEST in %s\n' "$legacy_cli" >&2
  exit 1
fi

rm -rf -- "$shim_cli_out"
mkdir -p "$shim_cli_out/bin"
sed \
  -e "s|^[[:space:]]*MANIFEST=.*|      MANIFEST=\${MANIFEST:-$manifest_path}|" \
  -e 's|^[[:space:]]*STATE_ROOT=.*|      STATE_ROOT=${STATE_ROOT:-/var/lib/nixling/vms}|' \
  "$legacy_cli" > "$shim_cli_out/bin/legacy-nixling"
chmod +x "$shim_cli_out/bin/legacy-nixling"
cat > "$shim_cli_out/bin/nixling" <<EOF_INTERCEPTED
#!/usr/bin/env bash
set -euo pipefail
MANIFEST_PATH=$manifest_path
STATE_ROOT=/var/lib/nixling/vms
SYSTEMCTL_BIN=systemctl
NATIVE_CLI=$native_cli
LEGACY_CLI=$shim_cli_out/bin/legacy-nixling
export MANIFEST="\$MANIFEST_PATH"
export STATE_ROOT="\$STATE_ROOT"
export NIXLING_MANIFEST_PATH="\$MANIFEST_PATH"
export NIXLING_STATE_ROOT="\$STATE_ROOT"
export NIXLING_LEGACY_CLI="\$LEGACY_CLI"
if [ "\$SYSTEMCTL_BIN" != "systemctl" ]; then
  PATH="\$(dirname "\$SYSTEMCTL_BIN"):\$PATH"
  export PATH
fi
case "\${1:-}" in
  status)
    if printf '%s\n' "\$@" | grep -Fx -- '--json' >/dev/null 2>&1; then
      "\$NATIVE_CLI" "\$@" | jq -c '{name, current, booted, pendingRestart, services}'
      exit \$?
    fi
    exec "\$LEGACY_CLI" "\$@"
    ;;
  audit)
    exec "\$LEGACY_CLI" "\$@"
    ;;
esac
exec "\$NATIVE_CLI" "\$@"
EOF_INTERCEPTED
chmod +x "$shim_cli_out/bin/nixling"
EOF_WRITE_SHIM_HELPER
chmod +x "$write_shim_helper"

cat > "$shim_path/nix-build" <<EOF_NIX_BUILD
#!/usr/bin/env bash
set -euo pipefail
legacy_out="\$("$real_nix_build" "\$@")"
"$write_shim_helper" "$cli" "\$legacy_out" "$shim_cli_out"
printf '%s\n' "$shim_cli_out"
EOF_NIX_BUILD
chmod +x "$shim_path/nix-build"

log "  dispatchable Layer-1 legacy tests: tests/cli-json.sh"
log "  deferred Layer-2/live-host tests: tests/{nixling-store,audio,network-isolation,audit-forwarding}.sh"

HOME="$scratch/home" XDG_RUNTIME_DIR="$scratch/runtime" \
NIXLING_LEGACY_CLI="$legacy" \
  "$cli" keys list --json > "$scratch/keys-via-shim.json"
HOME="$scratch/home" XDG_RUNTIME_DIR="$scratch/runtime" \
  "$legacy" keys list --json > "$scratch/keys-direct.json"
cmp -s "$scratch/keys-via-shim.json" "$scratch/keys-direct.json"
ok "legacy bash read-only commands still pass unchanged through the Rust shim"

run_legacy_suite() {
  local mode="$1" script_path="$2" script_name stdout_file stderr_file rc_file rc
  script_name=$(basename "$script_path")
  stdout_file="$scratch/$script_name.$mode.stdout"
  stderr_file="$scratch/$script_name.$mode.stderr"
  rc_file="$scratch/$script_name.$mode.rc"
  set +e
  if [ "$mode" = "direct" ]; then
    bash "$script_path" >"$stdout_file" 2>"$stderr_file"
    rc=$?
  else
    PATH="$shim_path:$PATH" bash "$script_path" >"$stdout_file" 2>"$stderr_file"
    rc=$?
  fi
  set -e
  printf '%s\n' "$rc" > "$rc_file"
}

run_legacy_suite direct "$HERE/cli-json.sh"
direct_rc=$(cat "$scratch/cli-json.sh.direct.rc")
if [ "$direct_rc" -ne 0 ]; then
  tail -80 "$scratch/cli-json.sh.direct.stderr" >&2 || true
  fail "cli-json.sh direct legacy baseline must pass before the shim parity rerun"
  exit 1
fi

run_legacy_suite shim "$HERE/cli-json.sh"
shim_rc=$(cat "$scratch/cli-json.sh.shim.rc")
if [ "$shim_rc" -ne "$direct_rc" ]; then
  tail -80 "$scratch/cli-json.sh.shim.stderr" >&2 || true
  fail "cli-json.sh exit status changed under the intercepted shim path ($direct_rc -> $shim_rc)"
  exit 1
fi
ok "cli-json.sh exits identically under the direct legacy path and the intercepted shim path"

cat > "$scratch/mock-legacy.sh" <<'EOF2'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$@" > "$MOCK_ARGV_FILE"
printf 'mock stdout\n'
printf 'mock stderr\n' >&2
exit 42
EOF2
chmod +x "$scratch/mock-legacy.sh"

set +e
MOCK_ARGV_FILE="$scratch/mock-argv.txt" \
NIXLING_LEGACY_CLI="$scratch/mock-legacy.sh" \
  "$cli" up corp-vm --detach > "$scratch/mock.stdout" 2> "$scratch/mock.stderr"
rc=$?
set -e

[ "$rc" -eq 42 ] || { fail "legacy dispatch must preserve the original exit code"; exit 1; }
if cmp -s "$scratch/mock.stdout" <(printf 'mock stdout\n') \
  && cmp -s "$scratch/mock.stderr" <(printf 'mock stderr\n'); then
  ok "legacy dispatch preserves stdout and stderr without extra chatter"
else
  fail "legacy dispatch altered stdout/stderr"
  exit 1
fi
if cmp -s "$scratch/mock-argv.txt" <(printf 'up\ncorp-vm\n--detach\n'); then
  ok "legacy dispatch preserves argv for top-level legacy subcommands"
else
  fail "legacy dispatch rewrote argv for top-level legacy subcommands"
  exit 1
fi

set +e
MOCK_ARGV_FILE="$scratch/mock-audit-argv.txt" \
NIXLING_LEGACY_CLI="$scratch/mock-legacy.sh" \
  "$cli" audit --strict --json > /dev/null 2> /dev/null
rc_audit=$?
set -e
[ "$rc_audit" -eq 42 ] || { fail "audit --strict legacy dispatch must preserve exit codes"; exit 1; }
if cmp -s "$scratch/mock-audit-argv.txt" <(printf 'audit\n--strict\n--json\n'); then
  ok "audit --strict dispatch preserves argv"
else
  fail "audit --strict dispatch rewrote argv"
  exit 1
fi

set +e
MOCK_ARGV_FILE="$scratch/mock-host-argv.txt" \
NIXLING_LEGACY_CLI="$scratch/mock-legacy.sh" \
NIXLING_TEST_DEPLOYMENT_SHAPE=tier0-all-legacy \
  "$cli" host prepare --apply > "$scratch/host-prepare-apply.stderr" 2>&1
rc_host=$?
set -e
# W3fu1 H1 (product-2): the Rust shim now refuses
# `host prepare --apply` on Tier-0 all-legacy with exit 78
# (`tier-0-legacy-uses-nixos-module`) per plan.md §"W3 per-tier
# host verb behavior" and never falls through to the legacy bash
# CLI. The mock-legacy mock argv file MUST stay empty.
[ "$rc_host" -eq 78 ] || { fail "host prepare --apply refused with exit 78, got $rc_host"; exit 1; }
if [ -s "$scratch/mock-host-argv.txt" ]; then
  fail "host prepare --apply must NOT pass through to the legacy CLI; mock argv was populated"
  cat "$scratch/mock-host-argv.txt" >&2
  exit 1
fi
ok "host prepare --apply refuses with exit 78 on Tier-0 all-legacy without falling through"

set +e
MOCK_ARGV_FILE="$scratch/mock-host-destroy-argv.txt" \
NIXLING_LEGACY_CLI="$scratch/mock-legacy.sh" \
NIXLING_TEST_DEPLOYMENT_SHAPE=tier0-all-legacy \
  "$cli" host destroy --apply > "$scratch/host-destroy-apply.stderr" 2>&1
rc_destroy=$?
set -e
[ "$rc_destroy" -eq 78 ] || { fail "host destroy --apply refused with exit 78, got $rc_destroy"; exit 1; }
if [ -s "$scratch/mock-host-destroy-argv.txt" ]; then
  fail "host destroy --apply must NOT pass through to the legacy CLI; mock argv was populated"
  exit 1
fi
ok "host destroy --apply refuses with exit 78 on Tier-0 all-legacy without falling through"

set +e
NIXLING_LEGACY_CLI="$scratch/mock-legacy.sh" \
  "$cli" host install > "$scratch/host-install.stdout" 2> "$scratch/host-install.stderr"
rc_install=$?
set -e
[ "$rc_install" -eq 70 ] || { fail "host install must return EX_SOFTWARE (70), got $rc_install"; exit 1; }
ok "host install returns not-yet-implemented exit 70 on every tier"

set +e
NIXLING_LEGACY_CLI="$scratch/mock-legacy.sh" \
  "$cli" host doctor > "$scratch/host-doctor.stdout" 2> "$scratch/host-doctor.stderr"
rc_doctor=$?
set -e
[ "$rc_doctor" -eq 78 ] || { fail "host doctor without --read-only must refuse with exit 78, got $rc_doctor"; exit 1; }
ok "host doctor without --read-only refuses with exit 78"

log "==> cli-legacy-bash-dispatch OK"
