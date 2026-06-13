#!/usr/bin/env bash
# Assert that every
# legacy systemd template / singleton the framework historically
# emitted has either a live doc row OR a documented retirement
# (obituary) in docs/reference/privileges.md— never both.
#
# Semantics
# ---------
# For each legacy unit pattern below, the gate inspects two doc
# regions:
#
#   * the **obituary region** = lines between the
#     `## final-pass: comprehensive legacy systemd surface obituary`
#     heading and the next top-level heading; this is the canonical
#     index of units the framework has retired
#   * the **live region**     = everything else in the doc; this is
#     the broker-op / runner-role / DAG-node surface that is the
#     daemon-only end-state
#
# Failure modes (hard fail):
#
#   (1) "in nixos-modules/ but undocumented" — a legacy unit name
#       still emitted by `nixos-modules/` that is mentioned nowhere
#       in the doc. Operators reading the doc cannot map the unit
#       to its current/future replacement.
#
#   (2) "deleted but undocumented" — a legacy unit name absent from
#       `nixos-modules/` AND absent from the obituary index.
#       Operators searching for the dead unit name find nothing.
#
#   (3) "self-contradictory: live row claims still-operational while
#        obituary claims deleted" — a legacy unit name appears in a
#       live (non-obituary) row WITHOUT any retirement marker AND
#       also appears in the obituary index. The doc tells operators
#       two opposite things. A live row whose text itself carries
#       the retirement marker (`Retired`, `deleted `, `Retired
#       (deleted in)`, etc.) is fine— that is the obituary
#       marker, not a contradiction.
#
# Transitional in-flight state ("emitted by nixos-modules/ AND
# documented in the obituary") is EXPECTED during the panel
# round: the doc lands first, the code-deletion sibling agent
# (``) lands next. The gate prints a
# WARNING for this state but does NOT fail; once
# `` ships, the warnings disappear and
# the gate is fully green on a clean post- tree.

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}
DOC=${DOC:-$ROOT/docs/reference/privileges.md}
MODULES_DIR=${MODULES_DIR:-$ROOT/nixos-modules}

# shellcheck source=lib.sh
# Intentionally NOT sourcing tests/lib.sh — this gate is pure
# string-manipulation on doc + nixos-modules trees and does not
# need the smoke-cache / cleanup-trap machinery, which obscures
# diagnostic output on early exits.

fail() { echo "FAIL: $*" >&2; exit 1; }
warn() { echo "WARN: $*" >&2; }

[[ -f "$DOC" ]]   || fail "privileges doc not found: $DOC"
[[ -d "$MODULES_DIR" ]] || fail "nixos-modules dir not found: $MODULES_DIR"

# Canonical legacy patterns. Each entry is a regex anchored on the
# unit base name; the same pattern matches `systemd.services.<x>`
# Nix attrs and `<x>.service`/`<x>@<vm>.service` doc citations.
LEGACY_UNITS=(
  # Per-VM templates (deleted by).
  'nixling@'
  'microvm@'
  'microvm-tap-interfaces@'
  'microvm-set-booted@'
  'microvm-pci-devices@'
  'microvm-virtiofsd@'
  'nixling-[^"@ ]+-gpu'
  'nixling-[^"@ ]+-video'
  'nixling-[^"@ ]+-snd'
  'nixling-[^"@ ]+-swtpm'
  'nixling-[^"@ ]+-store-sync'
  'nixling-known-hosts-refresh@'
  'nixling-vfsd-watchdog@'
  'nixling-otel-relay@'
  # Host singletons (deleted by).
  'nixling-net-route-preflight'
  'nixling-audit-check'
  'nixling-ch-exporter'
  'nixling-otel-host-bridge'
  'nixling-sys-[^"@ ]+-usbipd-proxy'
  'nixling-sys-[^"@ ]+-usbipd-backend'
)

OBIT_START=$(grep -n '^## Legacy systemd surface obituary' "$DOC" | head -1 | cut -d: -f1 || true)
[[ -n "$OBIT_START" ]] || fail "doc missing '## Legacy systemd surface obituary' section"
OBIT_END=$(awk -v s="$OBIT_START" 'NR>s && /^## / {print NR; exit}' "$DOC")
[[ -n "$OBIT_END" ]] || OBIT_END=$(wc -l < "$DOC")

# Read the doc ONCE into memory and pre-build the obituary region as a
# single string plus the live region as a line-indexed array. The
# per-pattern matching below then uses bash `[[ =~ ]]` and forks NO
# subprocesses. The previous design re-ran `sed`/`grep` over the whole
# doc for every legacy pattern (~60 forks); under peak static.sh memory
# pressure a single `sed` fork could fail, and with `set -o pipefail`
# the `if extract_obit | grep` test then silently evaluated false —
# misreading a present obituary row as absent and producing an
# intermittent false "no obituary row" failure (different unit each
# run). In-memory matching removes that fork-failure surface entirely.
mapfile -t DOC_LINES < "$DOC"
DOC_TOTAL=${#DOC_LINES[@]}
[[ "$DOC_TOTAL" -gt 0 ]] || fail "doc read produced no lines: $DOC"

# OBIT_START/OBIT_END are 1-based line numbers; arrays are 0-based.
OBIT_TEXT=""
for ((i = OBIT_START - 1; i < OBIT_END; i++)); do
  OBIT_TEXT+="${DOC_LINES[i]}"$'\n'
done
LIVE_LINES=()
for ((i = 0; i < DOC_TOTAL; i++)); do
  ln=$((i + 1))
  if ((ln >= OBIT_START && ln <= OBIT_END)); then
    continue
  fi
  LIVE_LINES+=("${DOC_LINES[i]}")
done

# A line in the live region carries an obituary marker if it
# mentions any of these phrases — they signal "this row is the
# obituary in-place, not a contradictory live row".
LIVE_OBIT_MARKERS='Retired|retired|retires|deleted|obituary|MUST NOT|scheduled.for.removal|folding their work|re-homed|replaced by|replacement|current surface|no longer exists|not emitted'

errors=0
warnings=0

for pat in "${LEGACY_UNITS[@]}"; do
  emitted=0
  if grep -rEq "systemd\\.services\\.\"?${pat}" "$MODULES_DIR" 2>/dev/null; then
    emitted=1
  fi

  # Doc citations must look like an actual systemd unit name —
  # require the pattern to abut a `.service`, `.socket`, `.timer`,
  # or `@<vm>` reference. Bare uid/principal mentions (e.g.
  # "nixling-<vm>-gpu uid") are not unit-name citations and don't
  # need an obituary marker.
  doc_pat="${pat}(\\.(service|socket|timer|\\{)|@|<vm>\\.)"

  in_obit=0
  [[ $OBIT_TEXT =~ $doc_pat ]] && in_obit=1

  # Count live-region mentions whose surrounding ±3 lines lack any
  # obituary marker. ±3 lines is enough to span the table-row line
  # plus the prose-paragraph context that introduces a retired-unit
  # list across multiple lines. All matching is in-memory (no fork).
  bare_live_hits=0
  in_live_any=0
  live_n=${#LIVE_LINES[@]}
  for ((li = 0; li < live_n; li++)); do
    [[ ${LIVE_LINES[li]} =~ $doc_pat ]] || continue
    in_live_any=1
    lo=$((li - 3)); ((lo < 0)) && lo=0
    hi=$((li + 3)); ((hi >= live_n)) && hi=$((live_n - 1))
    marked=0
    for ((wi = lo; wi <= hi; wi++)); do
      if [[ ${LIVE_LINES[wi]} =~ $LIVE_OBIT_MARKERS ]]; then
        marked=1
        break
      fi
    done
    ((marked == 0)) && bare_live_hits=$((bare_live_hits + 1))
  done

  # Failure (1): emitted by nixos-modules/ but undocumented.
  if [[ "$emitted" -eq 1 && "$in_live_any" -eq 0 && "$in_obit" -eq 0 ]]; then
    echo "FAIL: '$pat' is emitted by nixos-modules/ but mentioned nowhere in $DOC" >&2
    errors=$((errors + 1))
    continue
  fi

  # Failure (2): deleted but undocumented.
  if [[ "$emitted" -eq 0 && "$in_obit" -eq 0 ]]; then
    echo "FAIL: '$pat' is no longer emitted but has no obituary row" >&2
    errors=$((errors + 1))
    continue
  fi

  # Failure (3): self-contradictory live row + obituary.
  if [[ "$bare_live_hits" -gt 0 && "$in_obit" -eq 1 ]]; then
    echo "FAIL: '$pat' has a live (unmarked) doc row AND an obituary row — contradictory" >&2
    errors=$((errors + 1))
    continue
  fi

  # Transitional warning: still emitted AND already in the obituary.
  if [[ "$emitted" -eq 1 && "$in_obit" -eq 1 ]]; then
    warn "'$pat' still emitted by nixos-modules/ but already in obituary (transitional; systemd emission removal pending)"
    warnings=$((warnings + 1))
  fi
done

if [[ "$errors" -gt 0 ]]; then
  fail "$errors privileges-doc completeness violation(s) — see above"
fi

echo "OK: privileges-doc completeness gate passed (${#LEGACY_UNITS[@]} patterns; ${warnings} transitional warning(s))"
