#!/usr/bin/env bash
# W2 s3 Layer-1 gate: broker request dispositions stay closed and documented.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
DOC=${DOC:-$ROOT/docs/reference/broker-w2-dispositions.md}
SCHEMA=${SCHEMA:-$ROOT/docs/reference/schemas/v2/privileges.json}
SRC_DIR=${SRC_DIR:-$ROOT/packages/nixling-priv-broker/src}

if [ -z "${NIXLING_BROKER_ENUM_DISPOSITION_IN_NIX_SHELL:-}" ] && ! command -v python3 >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    echo "broker-enum-disposition: neither python3 nor nix is on PATH" >&2
    exit 1
  fi
  export NIXLING_BROKER_ENUM_DISPOSITION_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" nixpkgs#python3 --command bash "$0" "$@"
fi

python3 - "$DOC" "$SCHEMA" "$SRC_DIR" <<'PY'
import json
import re
import sys
from pathlib import Path


def fail(message: str) -> None:
    print(f"broker-enum-disposition: {message}", file=sys.stderr)
    raise SystemExit(1)


doc_path = Path(sys.argv[1])
schema_path = Path(sys.argv[2])
src_dir = Path(sys.argv[3])
dispatch_path = src_dir / "runtime.rs"
if not doc_path.is_file():
    fail(f"missing doc table: {doc_path}")
if not schema_path.is_file():
    fail(f"missing privileges schema: {schema_path}")
if not dispatch_path.is_file():
    fail(f"missing broker dispatcher: {dispatch_path}")

rows = {}
in_table = False
for raw_line in doc_path.read_text(encoding="utf-8").splitlines():
    line = raw_line.strip()
    if line.startswith("| Variant |"):
        in_table = True
        continue
    if in_table and not line.startswith("|"):
        break
    if not in_table or line.startswith("| ---"):
        continue
    parts = [part.strip() for part in line.strip("|").split("|")]
    if len(parts) != 4:
        fail(f"malformed table row: {raw_line}")
    variant, disposition, _note, _target = parts
    if variant in rows:
        fail(f"duplicate table row for {variant}")
    rows[variant] = disposition

schema = json.loads(schema_path.read_text(encoding="utf-8"))
expected = {
    op
    for op in schema["definitions"]["OperationAuthz"]["properties"]["operation"]["enum"]
    if op and op[0].isupper()
}
expected.add("Hello")

row_set = set(rows)
if row_set != expected:
    missing = sorted(expected - row_set)
    unexpected = sorted(row_set - expected)
    detail = []
    if missing:
        detail.append("missing rows: " + ", ".join(missing))
    if unexpected:
        detail.append("unexpected rows: " + ", ".join(unexpected))
    fail("; ".join(detail))

source_tree = "\n".join(
    path.read_text(encoding="utf-8") for path in sorted(src_dir.rglob("*.rs"))
)
dispatch_source = dispatch_path.read_text(encoding="utf-8")
arm_segments = {}
current_variant = None
current_lines = []
for line in dispatch_source.splitlines():
    stripped = line.lstrip()
    match = re.match(r"BrokerRequest::([A-Z][A-Za-z0-9]+)\b", stripped)
    if match:
        if current_variant is not None:
            arm_segments[current_variant] = "\n".join(current_lines)
        current_variant = match.group(1)
        current_lines = [line]
        continue
    if current_variant is not None:
        current_lines.append(line)
if current_variant is not None:
    arm_segments[current_variant] = "\n".join(current_lines)

for variant, disposition in sorted(rows.items()):
    if variant not in source_tree:
        fail(f"{variant} never appears under {src_dir}")

    segment = arm_segments.get(variant)
    if disposition == "callable-read-only":
        if segment is None:
            fail(f"callable variant {variant} is missing a dispatcher arm")
        if "BrokerError::Unimplemented" in segment:
            fail(f"callable variant {variant} still routes to BrokerError::Unimplemented")
    elif disposition == "stubbed-unimplemented":
        if segment is None:
            fail(f"stubbed variant {variant} is missing a dispatcher arm")
        if "BrokerError::Unimplemented" not in segment:
            fail(f"stubbed variant {variant} does not return BrokerError::Unimplemented")
    elif disposition == "stubbed-unknown-operation":
        # W3fu1 H1 (rust-1): W6 USBIP live device routing ops
        # (`UsbipBind`, `UsbipUnbind`, `UsbipProxyReconcile`) are
        # out of W3 scope and the W3 broker refuses them with
        # `BrokerError::UnknownOperation` so the audit shape
        # records `unknown-operation` instead of the `Unimplemented`
        # `w3-pending-typed-wire` shape used for genuinely deferred
        # in-scope variants. Doc + gate parity for that intent.
        if segment is None:
            fail(f"stubbed-unknown-operation variant {variant} is missing a dispatcher arm")
        if "BrokerError::UnknownOperation" not in segment:
            fail(
                f"stubbed-unknown-operation variant {variant} does not return "
                "BrokerError::UnknownOperation"
            )
    elif disposition == "compile-time-only":
        if segment is not None:
            fail(f"compile-time-only variant {variant} reached the wire dispatcher")
    else:
        fail(f"unknown disposition '{disposition}' for {variant}")

print("broker-enum-disposition: PASS")
PY
