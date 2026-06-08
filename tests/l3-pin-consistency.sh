#!/usr/bin/env bash
# tests/l3-pin-consistency.sh— distro matrix pin parser/drift gate.
#
# Enforces that distro matrix pin files are parsed and drift-checked.
#
# Asserts every file in tests/golden/l3-matrix/:
#   * exists,
#   * parses as a `key = value` ini-flavoured pin file,
#   * carries the required keys
#     (os/release/image_url/sha256/kernel_min/kernel_shipped/
#      cgroup/network_manager/nftables/cloud_hypervisor_min/minijail/
#      panel_approval_required_for_change),
#   * uses `placeholder` or a 64-char lowercase-hex sha256 (refuses
#     mojibake or partial digests),
#   * `panel_approval_required_for_change = true` (drift requires ADR).
#
# Scratch state lives outside $ROOT per AGENTS.md disk-hygiene contract.
#
# TODO(integrator): wire into tests/static.sh next to the existing
# `bash tests/path-safety-violation-fs.sh` invocation.

set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
# shellcheck source=lib.sh
. "$HERE/lib.sh"

cd "$ROOT"

PIN_DIR=${PIN_DIR:-$ROOT/tests/golden/l3-matrix}

if [ ! -d "$PIN_DIR" ]; then
  fail "l3-pin-consistency: pin directory missing: $PIN_DIR"
fi

REQUIRED_PINS=(w3-ubuntu.txt w3-fedora.txt w3-arch.txt)
REQUIRED_KEYS=(
  os release image_url sha256
  kernel_min kernel_shipped
  cgroup network_manager nftables
  cloud_hypervisor_min minijail
  panel_approval_required_for_change
)

log "W3 L3 pin parse + drift gate"

# Scratch outside $ROOT.
SCRATCH=${TMPDIR:-/tmp}/nixling-l3-pin.$$
mkdir -p "$SCRATCH"
add_cleanup "rm -rf -- '$SCRATCH'"

for pin in "${REQUIRED_PINS[@]}"; do
  path="$PIN_DIR/$pin"
  log " - $pin"
  if [ ! -f "$path" ]; then
    fail "l3-pin-consistency: required pin missing: $path"
  fi

  # Strip comments + blanks; the rest must be `key = value` lines.
  parsed="$SCRATCH/${pin}.parsed"
  : > "$parsed"
  while IFS= read -r line; do
    case "$line" in
      ''|'#'*) continue ;;
    esac
    # Reject lines that don't look like `key = value`.
    case "$line" in
      *=*) ;;
      *)
        fail "l3-pin-consistency: $pin: malformed line: $line"
        ;;
    esac
    key=${line%%=*}
    val=${line#*=}
    # Trim surrounding whitespace.
    key=$(printf '%s' "$key" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')
    val=$(printf '%s' "$val" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')
    case "$key" in
      ''|*[!a-z0-9_]*)
        fail "l3-pin-consistency: $pin: invalid key syntax: '$key'"
        ;;
    esac
    printf '%s=%s\n' "$key" "$val" >> "$parsed"
  done < "$path"

  for key in "${REQUIRED_KEYS[@]}"; do
    if ! grep -q "^${key}=" "$parsed"; then
      fail "l3-pin-consistency: $pin: missing required key '$key'"
    fi
  done

  sha=$(grep '^sha256=' "$parsed" | head -1 | cut -d= -f2-)
  if [ "$sha" != "placeholder" ]; then
    if ! printf '%s' "$sha" | grep -Eq '^[0-9a-f]{64}$'; then
      fail "l3-pin-consistency: $pin: sha256 must be 'placeholder' or 64-char lowercase hex, got: '$sha'"
    fi
  fi

  url=$(grep '^image_url=' "$parsed" | head -1 | cut -d= -f2-)
  case "$url" in
    https://*) ;;
    *) fail "l3-pin-consistency: $pin: image_url must be https://: '$url'" ;;
  esac

  panel=$(grep '^panel_approval_required_for_change=' "$parsed" | head -1 | cut -d= -f2-)
  if [ "$panel" != "true" ]; then
    fail "l3-pin-consistency: $pin: panel_approval_required_for_change must be 'true' (drift requires ADR), got: '$panel'"
  fi
done

ok "l3-pin-consistency: ${#REQUIRED_PINS[@]} pin files parse + carry required keys + sha256 syntax valid"
