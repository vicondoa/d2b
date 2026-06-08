#!/usr/bin/env bash
#  Runner-shape snapshot regression guards.
#
# Evals examples/minimal processesJson.data (with a test-only TPM overlay
# so the swtpm role is present), extracts argv and deviceBinds for:
#   cloud-hypervisor  — tests fu30 variadic --fs/--net/--disk/--device,
#                       fu31 absolute vsock socket path, and
#                       fu33 /dev/net/tun in mountPolicy.deviceBinds
#   virtiofsd         — tests fu14 ADR-0021 argv shape (--sandbox=chroot
#                       --inode-file-handles=never, absolute socket path)
#   swtpm             — tests long-lived swtpm argv (tpm.sock mode=0660)
#
# Compares against tests/fixtures/runner-shape-<role>.snap.
#
# Usage:
#   bash tests/runner-shape-snapshot.sh           # verify (default)
#   bash tests/runner-shape-snapshot.sh --update  # regenerate .snap files

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
export NL_LOG=${NL_LOG:-$ROOT/.runner-shape-snapshot.log}
export TMPDIR=${TMPDIR:-$ROOT/.copilot-work}
mkdir -p "$TMPDIR"

# shellcheck source=lib.sh
. "$HERE/lib.sh"

UPDATE=0
[ "${1:-}" = "--update" ] && UPDATE=1

FIXTURES="$ROOT/tests/fixtures"

# ---------------------------------------------------------------------------
# normalize: mask 32-char Nix content-addressed store hashes and
# host-specific /run/user/<uid> paths so snapshots survive rebuilds.
# ---------------------------------------------------------------------------
normalize() {
  sed \
    -e 's|/nix/store/[a-z0-9]\{32\}-|/nix/store/<HASH>-|g' \
    -e 's|/run/user/[0-9]\+|/run/user/<UID>|g'
}

# ---------------------------------------------------------------------------
# eval_processes_json: emit processesJson.data as JSON to stdout.
# Adds a test-only overlay enabling TPM so the swtpm role is present.
# Returns 75 when nix is not on PATH (caller treats as skip).
# Exits non-zero (rc≠75) on eval failure so the gate fails closed.
# ---------------------------------------------------------------------------
eval_processes_json() {
  if ! command -v nix >/dev/null 2>&1; then
    return 75
  fi
  nix eval --json --impure --no-warn-dirty \
    --expr "
      let
        root = builtins.getFlake (toString ${ROOT});
        nixos = root.inputs.nixpkgs.lib.nixosSystem {
          system = \"x86_64-linux\";
          modules = [
            root.nixosModules.default
            (import ${ROOT}/examples/minimal/configuration.nix)
            ({ ... }: {
              # Test-only overlay: enable TPM on the minimal VM so the
              # swtpm role appears in processesJson; does not affect the
              # CH or virtiofsd argv shape being tested.
              nixling.vms.personal-dev.tpm.enable = true;
            })
          ];
        };
      in nixos.config.nixling._bundle.processesJson.data
    " 2>>"$NL_LOG"
}

# ---------------------------------------------------------------------------
# extract_argv <json> <jq-role>
# Extract argv[] of the first node with the given role; one arg per line.
# ---------------------------------------------------------------------------
extract_argv() {
  local json="$1" jq_role="$2"
  printf '%s' "$json" \
    | jq -r '[.vms[].nodes[] | select(.role == "'"$jq_role"'")] | if length > 0 then .[0].argv[] else empty end' \
    | normalize
}

# ---------------------------------------------------------------------------
# extract_device_binds <json> <jq-role>
# Extract mountPolicy.deviceBinds[] of the first matching node.
# ---------------------------------------------------------------------------
extract_device_binds() {
  local json="$1" jq_role="$2"
  printf '%s' "$json" \
    | jq -r '[.vms[].nodes[] | select(.role == "'"$jq_role"'")] | if length > 0 then .[0].profile.mountPolicy.deviceBinds[] else empty end'
}

# ---------------------------------------------------------------------------
# build_snap <json> <role> <jq-role>
# Build the full snapshot text for a role.
# cloud-hypervisor also includes the deviceBinds section (fu33 guard).
# ---------------------------------------------------------------------------
build_snap() {
  local json="$1" role="$2" jq_role="$3"
  local argv
  argv=$(extract_argv "$json" "$jq_role")
  if [ "$role" = "cloud-hypervisor" ]; then
    local db
    db=$(extract_device_binds "$json" "$jq_role")
    printf '# argv\n%s\n# deviceBinds\n%s\n' "$argv" "$db"
  else
    printf '# argv\n%s\n' "$argv"
  fi
}

# ---------------------------------------------------------------------------
# compare_or_update <snap-path> <content> <label>
# ---------------------------------------------------------------------------
compare_or_update() {
  local snap="$1" content="$2" label="$3"
  if [ "$UPDATE" -eq 1 ]; then
    mkdir -p "$(dirname "$snap")"
    printf '%s\n' "$content" > "$snap"
    ok "runner-shape-snapshot: updated $snap"
    return 0
  fi
  if [ ! -f "$snap" ]; then
    fail "runner-shape-snapshot: missing $snap — run with --update to generate"
    return 1
  fi
  local expected
  expected=$(cat "$snap")
  if [ "$expected" = "$content" ]; then
    ok "runner-shape-snapshot: $label"
  else
    diff <(printf '%s\n' "$expected") <(printf '%s\n' "$content") >&2 || true
    fail "runner-shape-snapshot: $label drifted from $snap"
    return 1
  fi
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------
log "==> D15/P1.10 runner-shape snapshot tests (fu30/31/33)"

pjson_rc=0
pjson=$(eval_processes_json 2>>"$NL_LOG") || pjson_rc=$?

if [ "$pjson_rc" -eq 75 ]; then
  log "  SKIP: runner-shape-snapshot (nix not on PATH)"
  exit 0
elif [ "$pjson_rc" -ne 0 ]; then
  fail "runner-shape-snapshot: processesJson eval failed (rc=$pjson_rc) — see $NL_LOG"
  exit 1
fi

# role-name : jq-role-selector
for entry in \
  "cloud-hypervisor:cloud-hypervisor-runner" \
  "virtiofsd:virtiofsd" \
  "swtpm:swtpm"
do
  role="${entry%%:*}"
  jq_role="${entry##*:}"
  snap="$FIXTURES/runner-shape-${role}.snap"
  content=$(build_snap "$pjson" "$role" "$jq_role")
  compare_or_update "$snap" "$content" "$role"
done

log "D15/P1.10 runner-shape snapshot tests OK"
