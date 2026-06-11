#!/usr/bin/env bash
# tests/tempo-budget-eval.sh — legacy gate name retained for CI wiring.
#
# The Tempo retention/sampling backend was replaced by the native SigNoz
# observability backend. Keep this filename so existing workflow/static.sh
# references do not orphan a test, but assert the new OTel-native pipeline
# shape instead of the retired Tempo/Alloy contract.

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

STACK="$ROOT/nixos-modules/components/observability/stack.nix"
HOST_OPTS="$ROOT/nixos-modules/options-observability.nix"
ADR="$ROOT/docs/adr/0026-native-signoz-observability.md"

PASS=0
FAIL=0

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; PASS=$((PASS+1)); }
fail() { log "  FAIL: $*"; FAIL=$((FAIL+1)); }

log "==> tests/tempo-budget-eval.sh (native SigNoz compatibility gate)"

for f in "$STACK" "$HOST_OPTS" "$ADR"; do
  if [[ -f "$f" ]]; then
    ok "present: ${f#$ROOT/}"
  else
    fail "missing file: ${f#$ROOT/}"
  fi
done

if grep -q 'services\.clickhouse' "$STACK"; then
  ok "stack enables ClickHouse"
else
  fail "stack must enable ClickHouse for native SigNoz"
fi

if grep -q 'systemd\.services\.clickhouse-keeper' "$STACK"; then
  ok "stack enables ClickHouse Keeper as coordinator"
else
  fail "stack must enable ZooKeeper or a ClickHouse Keeper equivalent"
fi

for unit in signoz signoz-otel-collector signoz-schema-migrate-sync; do
  if grep -q "systemd.services.${unit}" "$STACK"; then
    ok "stack declares ${unit}.service"
  else
    fail "stack must declare ${unit}.service"
  fi
done

for token in 'signozspanmetrics/delta' 'memory_limiter' 'batch' 'clickhousetraces' 'clickhouselogsexporter' 'signozclickhousemetrics' 'metadataexporter'; do
  if grep -q "$token" "$STACK"; then
    ok "collector config contains $token"
  else
    fail "collector config missing $token"
  fi
done

for token in 'ingress.sources' 'sourceReceivers' 'sourceProcessors' 'sourcePipelines' 'nixling-otel-vsock-in-${sourceName}' 'resource/${sourceName}'; do
  if grep -q "$token" "$STACK"; then
    ok "collector uses source-specific ingress token $token"
  else
    fail "collector missing source-specific ingress token $token"
  fi
done

if grep -q '} // lib.optionalAttrs cfg\.scrapeNodeMetrics {' "$ROOT/nixos-modules/components/observability/guest.nix" \
  && ! grep -q 'hostmetrics = lib\.mkIf cfg\.scrapeNodeMetrics' "$ROOT/nixos-modules/components/observability/guest.nix"; then
  ok "guest collector conditionals are resolved before YAML serialization"
else
  fail "guest collector must not serialize lib.mkIf wrappers into OTel YAML"
fi

for token in 'prometheus/self' 'nixling-host-otel-collector' 'nixling-guest-otel-collector' 'metrics.readers'; do
  if grep -q "$token" "$STACK" "$ROOT/nixos-modules/components/observability/host.nix" "$ROOT/nixos-modules/components/observability/guest.nix"; then
    ok "collector self-telemetry token present: $token"
  else
    fail "collector self-telemetry token missing: $token"
  fi
done

if grep -q '"resource/self"' "$ROOT/nixos-modules/components/observability/guest.nix" \
  && grep -q 'pipelines\."metrics/self"' "$ROOT/nixos-modules/components/observability/guest.nix" \
  && ! grep -q '"pipelines.metrics/self"' "$ROOT/nixos-modules/components/observability/guest.nix"; then
  ok "guest collector self-metrics use a dedicated resource/self pipeline"
else
  fail "guest collector self-metrics must not share the workload resource processor"
fi

guest_workload_resource_block=$(
  sed -n '/resource.attributes = \[/,/\];/p' "$ROOT/nixos-modules/components/observability/guest.nix"
)
if [ -z "$guest_workload_resource_block" ]; then
  fail "guest workload resource processor block was not found"
elif grep -q 'service.name' <<<"$guest_workload_resource_block"; then
  fail "guest workload resource processor must preserve application service.name"
else
  ok "guest workload resource processor preserves application service.name"
fi

if grep -q 'pipelines = sourcePipelines' "$STACK" && ! grep -q 'receivers = \[ "otlp" \]' "$STACK"; then
  ok "collector pipelines are source-specific, not a shared otlp receiver"
else
  fail "collector must route through source-specific receiver pipelines"
fi

if grep -q '@uri' "$STACK" \
  && ! grep -q 'password=$pw"' "$STACK" \
  && ! grep -q 'password=$SIGNOZ_CLICKHOUSE_PASSWORD' "$STACK"; then
  ok "ClickHouse passwords are URL-encoded before DSN interpolation"
else
  fail "ClickHouse passwords embedded in DSN query strings must be URL-encoded"
fi

if grep -q '<password remove="1"/>' "$STACK" \
  && grep -q '<no_password/>' "$STACK" \
  && grep -q '<ip>127.0.0.1</ip>' "$STACK"; then
  ok "ClickHouse default user keeps a local-only auth method"
else
  fail "ClickHouse default user must not be left without an auth method"
fi

if ! grep -q -- '--manager-config' "$STACK" && ! grep -q 'conf/opamp.yaml' "$STACK"; then
  ok "SigNoz OTel collector runs the static nixling config without OpAMP manager mode"
else
  fail "SigNoz OTel collector must not enable OpAMP manager mode for static nixling receivers"
fi

if grep -q 'chmod 0444 "$file"' "$ROOT/nixos-modules/observability-host-secrets.nix" \
  && grep -q 'root:root 0700' "$ROOT/nixos-modules/observability-host-secrets.nix"; then
  ok "observability secrets are readable inside read-only virtiofs but protected by host parent dir"
else
  fail "observability secrets must be readable through the read-only virtiofs share"
fi

if grep -q 'otel_obs_connect_uids=.*vsock-relay' "$ROOT/nixos-modules/host-activation.nix" \
  && grep -q 'setfacl -d -m "u:$obs_uid:rw" "$obs_state_dir"' "$ROOT/nixos-modules/host-activation.nix" \
  && grep -q 'setfacl -m "u:$obs_uid:rw,m::rw" "$obs_vsock"' "$ROOT/nixos-modules/host-activation.nix"; then
  ok "workload OTLP relays inherit/connect to the obs VM vsock socket"
else
  fail "observed workload relay UIDs must get effective obs vsock socket ACLs"
fi

if grep -q '127\.0\.0\.1' "$STACK" && grep -q 'networking\.firewall\.allowedTCPPorts = \[ cfg\.signoz\.listenPort \]' "$STACK"; then
  ok "backend binds are loopback-oriented and only SigNoz UI port is opened"
else
  fail "stack must keep backend ports loopback-only and open only the SigNoz UI port"
fi

for retired in 'services\.grafana' 'services\.prometheus' 'services\.loki' 'services\.tempo' 'services\.alloy'; do
  if grep -q "$retired" "$STACK"; then
    fail "stack still declares retired backend ${retired}"
  else
    ok "stack does not declare retired backend ${retired}"
  fi
done

for option in 'signoz = {' 'listenPort' 'otlpGrpcPort' 'otlpHttpPort' 'adminEmail'; do
  if grep -q "$option" "$HOST_OPTS"; then
    ok "host options expose $option"
  else
    fail "host options missing $option"
  fi
done

if grep -q 'Spec corrections' "$ADR" && grep -q 'manifestVersion' "$ADR"; then
  ok "ADR records manifestVersion Spec corrections"
else
  fail "ADR must record manifestVersion Spec corrections"
fi

log "summary: PASS=$PASS FAIL=$FAIL"
if (( FAIL > 0 )); then
  exit 1
fi
