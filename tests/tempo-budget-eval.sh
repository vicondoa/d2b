#!/usr/bin/env bash
# tests/tempo-budget-eval.sh — static gate for the Tempo retention +
# sampling budget policy (P5 `ph5-p5-tempo-budget`).
#
# Asserts, against
# `nixos-modules/components/observability/stack.nix` and
# `nixos-modules/options-observability.nix` and
# `docs/reference/tempo-retention-sampling.md`:
#
#   1. The stack module pins the canonical retention + sampling
#      defaults (`retention.traces = "7d"`,
#      `retention.tracesCritical = "30d"`, `sampling.criticalRatio
#      = 1.0`, `sampling.defaultRatio = 0.1`,
#      `sampling.criticalAttribute = "kind"`,
#      `sampling.criticalValue = "critical"`,
#      `sampling.criticalTenant = "nixling-critical"`,
#      `sampling.defaultTenant = "nixling-default"`).
#   2. The host-side options mirror declares the same defaults for
#      every option the stack-side module declares (so consumer host
#      config sees the same surface).
#   3. The Tempo settings block enables multitenancy AND wires the
#      compactor ceiling to `tracesCritical` AND wires
#      `overrides.defaults.compaction.block_retention` to `traces`
#      AND points `per_tenant_override_config` at a generated
#      overrides file naming `sampling.criticalTenant`.
#   4. The Alloy traces pipeline contains a tail-sampling processor
#      with the two policies (critical=always, default=probabilistic
#      at `defaultRatio * 100`) and a routing connector splitting
#      by the `criticalAttribute`/`criticalValue` pair.
#   5. The canonical doc records the same numeric values + tenant
#      names. Drift between the Nix constants and the doc fails the
#      gate.
#
# Canonical contract: docs/reference/tempo-retention-sampling.md.
#
# Run via:
#   bash tests/tempo-budget-eval.sh

set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

STACK="$ROOT/nixos-modules/components/observability/stack.nix"
HOST_OPTS="$ROOT/nixos-modules/options-observability.nix"
DOC="$ROOT/docs/reference/tempo-retention-sampling.md"

PASS=0
FAIL=0

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; PASS=$((PASS+1)); }
fail() { log "  FAIL: $*"; FAIL=$((FAIL+1)); }

log "==> tests/tempo-budget-eval.sh"

for f in "$STACK" "$HOST_OPTS" "$DOC"; do
  if [[ ! -f "$f" ]]; then
    fail "missing file: $f"
  fi
done
(( FAIL > 0 )) && { log "summary: PASS=$PASS FAIL=$FAIL"; exit 1; }

# ---------------------------------------------------------------------------
# (1) stack.nix option defaults
# ---------------------------------------------------------------------------

# Each row: <option-path> <regex matching the default literal>
declare -a STACK_DEFAULTS=(
  'retention.traces|"7d"'
  'retention.tracesCritical|"30d"'
  'sampling.criticalAttribute|"kind"'
  'sampling.criticalValue|"critical"'
  'sampling.criticalRatio|1\.0'
  'sampling.defaultRatio|0\.1'
  'sampling.criticalTenant|"nixling-critical"'
  'sampling.defaultTenant|"nixling-default"'
)

# Extract option default for the given leaf name. Each option in
# stack.nix is declared via:
#   <leaf> = lib.mkOption {
#     type = ...;
#     default = <value>;
#     ...
#   };
# The defaults block we care about lives inside
# options.nixling.observability.{retention,sampling}.<leaf>.
extract_default() {
  local file=$1 leaf=$2
  awk -v leaf="$leaf" '
    $0 ~ "^[[:space:]]*" leaf "[[:space:]]*=[[:space:]]*lib\\.mkOption[[:space:]]*\\{" {
      in_opt = 1
      next
    }
    in_opt && /^[[:space:]]*default[[:space:]]*=/ {
      # capture everything after the `=` up to the trailing `;`
      sub(/^[[:space:]]*default[[:space:]]*=[[:space:]]*/, "")
      sub(/;[[:space:]]*$/, "")
      print
      in_opt = 0
      exit
    }
    in_opt && /^[[:space:]]*\}[[:space:]]*;/ {
      in_opt = 0
    }
  ' "$file"
}

check_stack_default() {
  local path=$1 want_re=$2
  local leaf=${path##*.}
  local got
  got=$(extract_default "$STACK" "$leaf")
  if [[ -z "$got" ]]; then
    fail "[$STACK] no default found for option '$path'"
    return
  fi
  if [[ "$got" =~ ^$want_re$ ]]; then
    ok "[$STACK] $path default = $got"
  else
    fail "[$STACK] $path default = '$got'; expected to match /^$want_re\$/"
  fi
}

for row in "${STACK_DEFAULTS[@]}"; do
  path=${row%%|*}
  want=${row#*|}
  check_stack_default "$path" "$want"
done

# ---------------------------------------------------------------------------
# (2) host-side mirror in options-observability.nix
# ---------------------------------------------------------------------------

check_host_default() {
  local path=$1 want_re=$2
  local leaf=${path##*.}
  local got
  got=$(extract_default "$HOST_OPTS" "$leaf")
  if [[ -z "$got" ]]; then
    fail "[$HOST_OPTS] no default found for option '$path'"
    return
  fi
  if [[ "$got" =~ ^$want_re$ ]]; then
    ok "[$HOST_OPTS] $path default = $got"
  else
    fail "[$HOST_OPTS] $path default = '$got'; expected /^$want_re\$/"
  fi
}

for row in "${STACK_DEFAULTS[@]}"; do
  path=${row%%|*}
  want=${row#*|}
  check_host_default "$path" "$want"
done

# ---------------------------------------------------------------------------
# (3) Tempo settings — multitenancy + per-tenant overrides
# ---------------------------------------------------------------------------

if grep -qE '^[[:space:]]*multitenancy_enabled[[:space:]]*=[[:space:]]*true;' "$STACK"; then
  ok "[$STACK] Tempo multitenancy_enabled = true"
else
  fail "[$STACK] Tempo multitenancy_enabled must be set to true"
fi

if grep -qE 'block_retention[[:space:]]*=[[:space:]]*cfg\.retention\.tracesCritical;' "$STACK"; then
  ok "[$STACK] compactor.block_retention pinned to cfg.retention.tracesCritical"
else
  fail "[$STACK] compactor.block_retention must reference cfg.retention.tracesCritical (the global ceiling)"
fi

if grep -qE 'block_retention[[:space:]]*=[[:space:]]*cfg\.retention\.traces;' "$STACK"; then
  ok "[$STACK] overrides.defaults.compaction.block_retention pinned to cfg.retention.traces"
else
  fail "[$STACK] overrides.defaults.compaction.block_retention must reference cfg.retention.traces"
fi

if grep -qE 'per_tenant_override_config[[:space:]]*=[[:space:]]*toString[[:space:]]+tempoPerTenantOverrides;' "$STACK"; then
  ok "[$STACK] per_tenant_override_config wired to generated overrides file"
else
  fail "[$STACK] per_tenant_override_config must be wired to the generated tempoPerTenantOverrides file"
fi

if grep -qE 'tempoPerTenantOverrides[[:space:]]*=[[:space:]]*pkgs\.writeText' "$STACK"; then
  ok "[$STACK] tempoPerTenantOverrides is generated via pkgs.writeText"
else
  fail "[$STACK] tempoPerTenantOverrides must be generated via pkgs.writeText"
fi

if grep -qE '"\$\{cfg\.sampling\.criticalTenant\}"[[:space:]]*=' "$STACK"; then
  ok "[$STACK] tempoPerTenantOverrides keys on cfg.sampling.criticalTenant"
else
  fail "[$STACK] tempoPerTenantOverrides must key on cfg.sampling.criticalTenant"
fi

# ---------------------------------------------------------------------------
# (4) Alloy pipeline — tail_sampling + routing connector
# ---------------------------------------------------------------------------

if grep -q 'otelcol.processor.tail_sampling "tempo_budget"' "$STACK"; then
  ok "[$STACK] Alloy declares otelcol.processor.tail_sampling.tempo_budget"
else
  fail "[$STACK] Alloy must declare otelcol.processor.tail_sampling.tempo_budget"
fi

if grep -q 'otelcol.connector.routing "tempo_tenant"' "$STACK"; then
  ok "[$STACK] Alloy declares otelcol.connector.routing.tempo_tenant"
else
  fail "[$STACK] Alloy must declare otelcol.connector.routing.tempo_tenant"
fi

if grep -q 'otelcol.exporter.otlp "traces_critical"' "$STACK" \
  && grep -q 'otelcol.exporter.otlp "traces_default"' "$STACK"; then
  ok "[$STACK] Alloy declares both traces_critical and traces_default exporters"
else
  fail "[$STACK] Alloy must declare both traces_{critical,default} OTLP exporters"
fi

# The receiver must forward to the tail-sampling processor (not the
# pre-P5 single exporter).
if grep -qE 'traces[[:space:]]*=[[:space:]]*\[otelcol\.processor\.tail_sampling\.tempo_budget\.input\]' "$STACK"; then
  ok "[$STACK] Alloy otlp ingress forwards traces to tail_sampling.tempo_budget"
else
  fail "[$STACK] Alloy otlp ingress must forward traces to tail_sampling.tempo_budget.input"
fi

# Sampling percentage in the default-probabilistic policy must match
# defaultRatio * 100. Computed from extract_default (1.0/0.1 form).
default_ratio=$(extract_default "$STACK" defaultRatio)
default_pct=""
case "$default_ratio" in
  0.1)  default_pct="10" ;;
  1.0)  default_pct="100" ;;
  0.0)  default_pct="0" ;;
  *)    default_pct=$(awk -v r="$default_ratio" 'BEGIN { printf "%g", r * 100 }') ;;
esac
if grep -qE "sampling_percentage[[:space:]]*=[[:space:]]*\\\$\\{samplingPercentageDefault\\}" "$STACK"; then
  ok "[$STACK] default_probabilistic policy interpolates samplingPercentageDefault (= $default_pct)"
else
  fail "[$STACK] default_probabilistic policy must use \${samplingPercentageDefault} (computed = $default_pct)"
fi

# ---------------------------------------------------------------------------
# (5) Doc drift — the canonical doc must record the same numeric
#     values + tenant names + attribute pair.
# ---------------------------------------------------------------------------

declare -a DOC_TOKENS=(
  '7 days'
  '30 days'
  '100 %'
  '10 %'
  'nixling-critical'
  'nixling-default'
  'kind="critical"'
  'retention.tracesCritical'
  'retention.traces'
  'sampling.criticalAttribute'
  'sampling.criticalValue'
  'sampling.criticalRatio'
  'sampling.defaultRatio'
  'sampling.criticalTenant'
  'sampling.defaultTenant'
  'multitenancy_enabled'
  'per_tenant_override_config'
  'tail_sampling.tempo_budget'
  'routing.tempo_tenant'
  'tempo-critical'
)

for tok in "${DOC_TOKENS[@]}"; do
  if grep -qF -- "$tok" "$DOC"; then
    ok "[$DOC] mentions '$tok'"
  else
    fail "[$DOC] must mention '$tok' (drift between Nix policy + doc)"
  fi
done

# ---------------------------------------------------------------------------
log "summary: PASS=$PASS FAIL=$FAIL"
if (( FAIL > 0 )); then
  exit 1
fi
exit 0
