#!/usr/bin/env bash
# tests/loki-label-cardinality-eval.sh — compatibility filename for the
# retired Loki label gate.
#
# Current purpose: static gate for the native SigNoz/OpenTelemetry
# resource-attribute contract. It keeps the historical filename so
# static.sh / workflow wiring and ci-coverage do not churn, but the live
# assertions are about OTel collectors, not Loki.

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

FILES=(
  "$ROOT/nixos-modules/components/observability/host.nix"
  "$ROOT/nixos-modules/components/observability/stack.nix"
  "$ROOT/nixos-modules/components/observability/guest.nix"
)

ALLOWED_RESOURCE_KEYS=(
  deployment.environment
  host.name
  service.name
  service.namespace
  vm.env
  vm.name
  vm.role
)

REQUIRED_RESOURCE_KEYS=(
  service.name
  vm.env
  vm.name
  vm.role
)

FORBIDDEN_VALUE_RE='(secret|password|token|private[_-]?key|argv|cmdline|command[_-]?line|stdout|stderr|/nix/store)'

PASS=0
FAIL=0

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; PASS=$((PASS+1)); }
fail() { log "  FAIL: $*"; FAIL=$((FAIL+1)); }

in_list() {
  local needle=$1; shift
  local hay
  for hay in "$@"; do
    [[ "$hay" == "$needle" ]] && return 0
  done
  return 1
}

log "==> tests/loki-label-cardinality-eval.sh (native OTel resource attributes)"

for file in "${FILES[@]}"; do
  if [[ -f "$file" ]]; then
    ok "present: ${file#$ROOT/}"
  else
    fail "missing file: ${file#$ROOT/}"
  fi
done

if grep -R -n -E 'loki\.source|services\.(alloy|loki|tempo|prometheus|grafana)' "${FILES[@]}" >/tmp/nixling-retired-observability.$$ 2>/dev/null; then
  fail "retired Loki/Alloy/Grafana stack references remain in live observability modules"
  sed 's/^/    /' /tmp/nixling-retired-observability.$$ >&2 || true
else
  ok "live observability modules do not emit retired Loki/Alloy/Grafana stack surfaces"
fi
rm -f /tmp/nixling-retired-observability.$$

mapfile -t observed_keys < <(
  grep -RhoE 'key[[:space:]]*=[[:space:]]*"[^"]+"' "${FILES[@]}" \
    | sed -E 's/.*"([^"]+)".*/\1/' \
    | LC_ALL=C sort -u
)

if ((${#observed_keys[@]} == 0)); then
  fail "no OTel resource attribute keys found"
else
  ok "found ${#observed_keys[@]} distinct OTel resource attribute keys"
fi

for key in "${observed_keys[@]}"; do
  if in_list "$key" "${ALLOWED_RESOURCE_KEYS[@]}"; then
    ok "resource attribute key allowed: $key"
  else
    fail "resource attribute key '$key' is outside the bounded allowlist"
  fi
done

for key in "${REQUIRED_RESOURCE_KEYS[@]}"; do
  if printf '%s\n' "${observed_keys[@]}" | grep -Fxq "$key"; then
    ok "required resource attribute key present: $key"
  else
    fail "required resource attribute key missing: $key"
  fi
done

if grep -R -n -E "key[[:space:]]*=[[:space:]]*\"[^\"]*${FORBIDDEN_VALUE_RE}[^\"]*\"" "${FILES[@]}" >/tmp/nixling-forbidden-resource-keys.$$ 2>/dev/null; then
  fail "forbidden sensitive/high-cardinality resource attribute keys found"
  sed 's/^/    /' /tmp/nixling-forbidden-resource-keys.$$ >&2 || true
else
  ok "resource attribute keys avoid secrets, argv, command output, and store paths"
fi
rm -f /tmp/nixling-forbidden-resource-keys.$$

for token in 'action = "upsert"' 'vm.name' 'vm.env' 'vm.role' 'service.name'; do
  if grep -R -q "$token" "${FILES[@]}"; then
    ok "collector config contains $token"
  else
    fail "collector config missing $token"
  fi
done

log "summary: PASS=$PASS FAIL=$FAIL"
if (( FAIL > 0 )); then
  exit 1
fi
