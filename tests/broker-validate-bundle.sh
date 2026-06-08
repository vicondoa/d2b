#!/usr/bin/env bash
# The broker delegates bundle validation to nixling-core only.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
SRC_DIR=${SRC_DIR:-$ROOT/packages/nixling-priv-broker/src}

if [ ! -d "$SRC_DIR" ]; then
  echo "broker-validate-bundle: missing broker source directory: $SRC_DIR" >&2
  exit 1
fi

if ! grep -R -q 'nixling_core::manifest' "$SRC_DIR"; then
  echo "broker-validate-bundle: expected nixling_core::manifest import under $SRC_DIR" >&2
  exit 1
fi

if ! grep -R -q 'validate_bundle' "$SRC_DIR"; then
  echo "broker-validate-bundle: expected validate_bundle call under $SRC_DIR" >&2
  exit 1
fi

if grep -R -nE 'serde_json::from_(str|value)' "$SRC_DIR" >/dev/null; then
  grep -R -nE 'serde_json::from_(str|value)' "$SRC_DIR" >&2 || true
  echo "broker-validate-bundle: duplicate broker-side JSON parsing detected" >&2
  exit 1
fi

echo "broker-validate-bundle: PASS"
