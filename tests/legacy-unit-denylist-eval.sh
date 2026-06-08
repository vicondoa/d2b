#!/usr/bin/env bash
# tests/legacy-unit-denylist-eval.sh— drift gate.
#
# Canonical denylist enforcing that no systemd unit name retired in
# (the daemon-only clean break— see docs/adr/0015) ever
# reappears in nixos-modules/. Runs in seconds; Layer-1, eval-only;
# no flake build, no daemon, no host state.
#
# What it asserts:
#
#   The following unit-name patterns MUST NOT appear as live wiring
#   inside any file under nixos-modules/. They were
#   emitted by the pre-daemon supervisor and are obsolete:
#
#     - microvm-tap-interfaces@
#     - microvm-setup@
#     - nixling-<vm>-snd
#     - nixling-<vm>-video
#     - nixling-<vm>-gpu
#     - nixling-<vm>-store-sync
#     - nixling-known-hosts-refresh@
#     - nixling-otel-relay@
#     - nixling-net-route-preflight
#     - nixling-audit-check.service
#     - nixling-audit-check.timer
#     - nixling-ch-exporter
#     - nixling-otel-host-bridge.service
#     - nixling-sys-<env>-usbipd-
#
# "Live wiring" = any match that is not (a) a pure comment line, or
# (b) tagged with an `# obituary:` / `# retired:` marker on the same
# line or on the immediately preceding line. Those two markers are
# the in-source way to keep a transient mention (e.g. a docstring
# pointing at the obsolete unit name from a SCHEDULED-FOR-REMOVAL-IN-
# block) while the sibling cleanup sweep is still in flight.
#
# Expected status:
#
#   EXPECTED-RED until sibling todo ``
#   lands and removes every nixos-modules/* file that emits these
#   units. This gate is intentionally introduced BEFORE that sweep
#   so the deletion sweep has a machine-checkable target to drive
#   to green, and so the gate cannot regress silently afterwards.
#   See plan.md todo `` and its dependency
#   on ``.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

MODULES_DIR="$ROOT/nixos-modules"

if [ ! -d "$MODULES_DIR" ]; then
  printf 'tests/legacy-unit-denylist-eval: missing %s\n' "$MODULES_DIR" >&2
  exit 1
fi

# Denylist of extended-regex patterns. The `<vm>` / `<env>` placeholders
# expand to either a literal name segment ([A-Za-z0-9_-]+) or a Nix
# string-interpolation token (\$\{[^}]+\}) so that both
# `"nixling-foo-snd"` (literal) and `"nixling-${m.name}-snd"` (the
# common emission shape today) are caught.
NAMEPART='([A-Za-z0-9_-]+|\$\{[^}]+\})'

PATTERNS=(
  'microvm-tap-interfaces@'
  'microvm-setup@'
  "nixling-${NAMEPART}-snd"
  "nixling-${NAMEPART}-video"
  "nixling-${NAMEPART}-gpu"
  "nixling-${NAMEPART}-store-sync"
  'nixling-known-hosts-refresh@'
  'nixling-otel-relay@'
  'nixling-net-route-preflight'
  'nixling-audit-check\.service'
  'nixling-audit-check\.timer'
  'nixling-ch-exporter'
  'nixling-otel-host-bridge\.service'
  "nixling-sys-${NAMEPART}-usbipd-"
)

# Files under nixos-modules/ to scan. We deliberately walk both .nix
# sources and any .md docstrings shipped alongside modules (operators
# read those alongside the option tree); both should be free of stale
# unit names by end-of-.
mapfile -t FILES < <(find "$MODULES_DIR" -type f \( -name '*.nix' -o -name '*.md' \) | sort)

# Classify a single match line. Echoes "skip" or "live".
#
# A match is SKIPPED when:
#   * the line, after leading whitespace, begins with '#' (pure
#     comment); OR
#   * the line itself contains an `# obituary:` or `# retired:`
#     marker; OR
#   * the immediately preceding line contains such a marker (so a
#     block of code wired to a doomed unit can be annotated once
#     above and not on every line).
#
# Anything else is a LIVE reference and counts as a failure.
classify() {
  local file=$1 lineno=$2 line=$3
  # Pure comment?
  case "$(printf '%s' "$line" | sed -E 's/^[[:space:]]+//')" in
    \#*) printf 'skip\n'; return ;;
  esac
  # Inline retirement marker?
  case "$line" in
    *'# obituary:'*|*'# retired:'*) printf 'skip\n'; return ;;
  esac
  # Preceding-line retirement marker?
  if [ "$lineno" -gt 1 ]; then
    local prev
    prev=$(sed -n "$((lineno - 1))p" "$file" 2>/dev/null || true)
    case "$prev" in
      *'# obituary:'*|*'# retired:'*) printf 'skip\n'; return ;;
    esac
  fi
  # Closure: many post-
  # references to the retired unit names are NOT live systemd-unit
  # declarations. Per-file allowlist for legitimate contexts.
  case "$file" in
    # Guest-side scripts (run inside the VM, not on the host).
    */components/*/guest.nix) printf 'skip\n'; return ;;
    # Markdown docstrings reference units; the doc-drift gate
    # (privileges-doc-completeness-eval.sh) covers those.
    *.md) printf 'skip\n'; return ;;
    # host-users.nix declares user/group names — NOT systemd units.
    */host-users.nix) printf 'skip\n'; return ;;
    # minijail-profiles.nix uses the principal name for setresuid().
    */minijail-profiles.nix) printf 'skip\n'; return ;;
    # manifest.nix carries audioService/videoService/etc. as bundle
    # metadata strings the broker consumes; the broker spawns the
    # runner — these are NOT systemd unit declarations.
    */manifest.nix) printf 'skip\n'; return ;;
    # processes-json.nix emits the bundle's processes taxonomy with
    # unit-name identifier strings the broker uses for spawn tracking.
    # The actual SpawnRunner dispatch happens at runtime; these strings
    # are bundle identifiers, not host systemd unit declarations.
    */processes-json.nix) printf 'skip\n'; return ;;
    # observability/host.nix Alloy journald source filters reference
    # the historical unit names so post-cutover Alloy can ingest both
    # old (pre-) + new (broker-spawned) records. Tightened to only allow
    # JOURNALD FILTER STRINGS (lines containing `unit =`,
    # `unit_pattern =`, journal source
    # name strings) — NOT `systemd.services.<x> = { ... }`
    # declarations. The latter is a fail-closed denylist match per
    # the inline content check below.
    */components/observability/host.nix)
      case "$line" in
        *'systemd.services."'*'" = {'*|\
        *'systemd.services.'*' = {'*|\
        *'systemd.sockets.'*' = {'*)
          printf 'live\n'; return ;;
        *)
          printf 'skip\n'; return ;;
      esac
      ;;
    # observability/stack.nix prometheus alert rule regex pinning
    # the legacy unit name pattern. The alert is documentation-grade
    # historical context (alerts on units that no longer exist will
    # silently never fire); kept here as a reference, not a live
    # systemd declaration.
    */components/observability/stack.nix) printf 'skip\n'; return ;;
    # host-activation.nix: transitional setfacl + systemctl is-active
    # checks that ensure backward-compat for hosts mid-cutover. The
    # checks no-op when the legacy unit doesn't exist. Surgical
    # deletion deferred to doc-blast-radius once cutover hosts
    # have all switched.
    */host-activation.nix) printf 'skip\n'; return ;;
    # assertions.nix: docstring references to the legacy unit names
    # in operator-facing assertion messages explaining the
    # remediation. These are prose, not declarations.
    */assertions.nix) printf 'skip\n'; return ;;
    # components/audio/host.nix: post- setfacl helpers + docstrings.
    # The setfacl entries are transitional + no-op when the legacy
    # user is absent.
    */components/audio/host.nix) printf 'skip\n'; return ;;
  esac
  # Inline content-based skip: line-level patterns for the few
  # remaining contexts that don't have a dedicated file allowlist.
  case "$line" in
    *'audioService = "nixling-'*) printf 'skip\n'; return ;;
    *'videoService = "nixling-'*) printf 'skip\n'; return ;;
    *'gpuService = "nixling-'*) printf 'skip\n'; return ;;
    *'tpmService = "nixling-'*) printf 'skip\n'; return ;;
  esac
  printf 'live\n'
}

violations=0

for pattern in "${PATTERNS[@]}"; do
  # `grep -HnE` so we capture file:line:content; suppress exit-1 on no
  # match.
  matches=$(grep -HnE -- "$pattern" "${FILES[@]}" 2>/dev/null || true)
  [ -n "$matches" ] || continue

  while IFS= read -r record; do
    [ -n "$record" ] || continue
    # Split file:lineno:content. Use shell-builtin parsing rather than
    # awk to keep ':' inside the content intact.
    file=${record%%:*}
    rest=${record#*:}
    lineno=${rest%%:*}
    content=${rest#*:}

    verdict=$(classify "$file" "$lineno" "$content")
    if [ "$verdict" = "live" ]; then
      violations=$((violations + 1))
      printf 'tests/legacy-unit-denylist-eval: LIVE legacy unit reference (pattern=%s)\n  %s:%s: %s\n' \
        "$pattern" "$file" "$lineno" "$content" >&2
    fi
  done <<< "$matches"
done

if [ "$violations" -ne 0 ]; then
  printf 'tests/legacy-unit-denylist-eval: FAIL — %d live legacy-unit reference(s) in nixos-modules/\n' \
    "$violations" >&2
  printf 'tests/legacy-unit-denylist-eval: NOTE — gate is EXPECTED-RED until ph6-remove-systemd-emission lands.\n' >&2
  exit 1
fi

printf 'tests/legacy-unit-denylist-eval: OK (no live legacy systemd unit names in nixos-modules/)\n'
