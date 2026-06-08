#!/usr/bin/env bash
# v1.1 invariant gate: assert host-otel-relay-acl.nix is no
# longer imported via the public default.nix entry point. The OTel
# host-bridge + per-VM relay ACL contract migrated into the broker
# pre-spawn pipeline at `packages/nixling-priv-broker/src/runtime.rs`
# (the `SpawnRunner{role: OtelHostBridge}` handler — already in v1.0
# source). The retired NixOS-side module is kept as a stub for one
# release for diff readability; this gate enforces it is NOT in the
# public import list.
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

defaults="$ROOT/nixos-modules/default.nix"

# Match a NON-commented import. The line must not start with `#`
# after optional whitespace.
if grep -E '^\s*\./host-otel-relay-acl\.nix' "$defaults" >/dev/null 2>&1; then
  printf 'otel-acl-migration-eval: FAIL — host-otel-relay-acl.nix still imported by %s\n' "$defaults" >&2
  exit 1
fi

# Also assert the broker's OtelHostBridge handler is present so the
# migration is meaningful (the systemd surface is replaced by the
# broker SpawnRunner pipeline).
broker_runtime="$ROOT/packages/nixling-priv-broker/src/runtime.rs"
if ! grep -q -E 'RunnerRole::OtelHostBridge' "$broker_runtime"; then
  printf 'otel-acl-migration-eval: FAIL — broker runtime missing OtelHostBridge handler in %s\n' "$broker_runtime" >&2
  exit 1
fi

printf 'otel-acl-migration-eval: PASS\n'
