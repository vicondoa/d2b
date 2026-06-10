#!/usr/bin/env bash
# Validate the checked-in guest-control protobuf source compiles to a descriptor.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

proto_dir="$ROOT/packages/nixling-ipc/proto"
proto="$proto_dir/guest_control.proto"
scratch=$(nl_mktemp .guest-control-proto.XXXXXX)
descriptor="$scratch/guest_control.pb"

if [ ! -f "$proto" ]; then
  fail "guest-control-proto: missing $proto"
fi

if command -v protoc >/dev/null 2>&1; then
  protoc_cmd=(protoc)
else
  protoc_cmd=(nix shell --quiet --inputs-from "$ROOT" nixpkgs#protobuf --command protoc)
fi

"${protoc_cmd[@]}" \
  --proto_path="$proto_dir" \
  --include_source_info \
  --descriptor_set_out="$descriptor" \
  "$proto"

if [ ! -s "$descriptor" ]; then
  fail "guest-control-proto: empty descriptor output"
fi

decoded="$scratch/guest_control.pb.txt"
"${protoc_cmd[@]}" \
  --decode=google.protobuf.FileDescriptorSet \
  google/protobuf/descriptor.proto \
  < "$descriptor" > "$decoded"

require_descriptor() {
  local pattern="$1" message="$2"
  if ! grep -q -- "$pattern" "$decoded"; then
    fail "guest-control-proto: descriptor missing $message"
  fi
}

reject_descriptor() {
  local pattern="$1" message="$2"
  if grep -q -- "$pattern" "$decoded"; then
    fail "guest-control-proto: descriptor unexpectedly contains $message"
  fi
}

require_descriptor 'name: "GuestControl"' "GuestControl service"
for method in \
  Hello Capabilities Health ExecCreate ExecInspect ExecWait ExecLogs \
  WriteStdin ReadOutput CloseStdin TtyWinResize ExecSignal ExecCancel
do
  require_descriptor "name: \"$method\"" "method $method"
done

for field in \
  guest_boot_id pending_read_output_waits_per_stream pending_exec_waits_per_vm \
  rpc_rate_per_connection_per_second rpc_rate_per_vm_burst end_offset \
  timed_out retry_after_ms
do
  require_descriptor "name: \"$field\"" "field $field"
done

optional_count=$(grep -c 'proto3_optional: true' "$decoded" || true)
if [ "$optional_count" -lt 6 ]; then
  fail "guest-control-proto: expected optional scalar/string fields in descriptor"
fi

require_descriptor 'name: "outcome"' "TerminalStatus outcome oneof"
require_descriptor 'name: "WRITE_DISPOSITION_REJECTED"' "rejected stdin disposition"
require_descriptor 'name: "GUEST_CONTROL_ERROR_KIND_STDIN_OFFSET_MISMATCH"' "stdin offset mismatch error"
require_descriptor 'name: "GUEST_CONTROL_ERROR_KIND_OFFSET_IN_FUTURE"' "offset-in-future error"
require_descriptor 'name: "GUEST_CONTROL_ERROR_KIND_RETAINED_LOG_PATH_UNSAFE"' "retained-log path error"
require_descriptor 'name: "GUEST_CONTROL_ERROR_KIND_RETAINED_LOG_QUOTA_EXCEEDED"' "retained-log quota error"
require_descriptor 'name: "GUEST_CONTROL_ERROR_KIND_CWD_INVALID"' "cwd-invalid error"
require_descriptor 'name: "GUEST_CONTROL_ERROR_KIND_CWD_DENIED"' "cwd-denied error"
reject_descriptor 'name: "GUEST_CAPABILITY_READ_GUEST_CONFIG"' "unbacked ReadGuestConfig capability"
reject_descriptor 'name: "SIGNAL_TARGET_ROOT_PROCESS"' "ungated root-process signal target"

schema="$ROOT/docs/reference/schemas/v2/guest-control.json"
if [ ! -f "$schema" ]; then
  fail "guest-control-proto: missing generated schema $schema"
fi

if command -v python3 >/dev/null 2>&1; then
  python_cmd=(python3)
else
  python_cmd=(nix shell --quiet --inputs-from "$ROOT" nixpkgs#python3 --command python3)
fi

"${python_cmd[@]}" - "$proto" "$schema" <<'PY'
import json
import re
import sys
from pathlib import Path

proto = Path(sys.argv[1]).read_text(encoding="utf-8")
schema = json.loads(Path(sys.argv[2]).read_text(encoding="utf-8"))
defs = schema.get("definitions", {})


def blocks(kind):
    pattern = re.compile(rf"^{kind}\s+(\w+)\s*\{{", re.MULTILINE)
    for match in pattern.finditer(proto):
        name = match.group(1)
        idx = match.end()
        depth = 1
        end = idx
        while end < len(proto) and depth:
            if proto[end] == "{":
                depth += 1
            elif proto[end] == "}":
                depth -= 1
            end += 1
        yield name, proto[idx : end - 1]


message_fields = {}
optional_fields = {}
for name, body in blocks("message"):
    fields = []
    optional = set()
    for line in body.splitlines():
        line = line.strip()
        match = re.match(r"(optional\s+)?(?:repeated\s+)?[.\w]+\s+(\w+)\s*=\s*\d+\s*;", line)
        if match:
            field = match.group(2)
            fields.append(field)
            if match.group(1):
                optional.add(field)
    message_fields[name] = fields
    optional_fields[name] = optional


enum_values = {}
for name, body in blocks("enum"):
    values = []
    for line in body.splitlines():
        line = line.strip()
        match = re.match(r"([A-Z0-9_]+)\s*=\s*\d+\s*;", line)
        if match:
            values.append(match.group(1))
    enum_values[name] = values


def camel(name):
    head, *tail = name.split("_")
    return head + "".join(part.capitalize() for part in tail)


message_map = {
    "RequestMetadata": "GuestRequestMetadata",
    "ExecRequestMetadata": "GuestExecRequestMetadata",
}

skip_messages = {"TerminalStatus"}
for proto_name, fields in sorted(message_fields.items()):
    if proto_name in skip_messages:
        continue
    schema_name = message_map.get(proto_name, proto_name)
    node = defs.get(schema_name)
    if node is None:
        raise SystemExit(f"schema missing definition for proto message {proto_name}")
    props = set(node.get("properties", {}).keys())
    wanted = {camel(field) for field in fields}
    if props != wanted:
        raise SystemExit(
            f"{proto_name}/{schema_name} field drift: schema={sorted(props)} proto={sorted(wanted)}"
        )
    required = set(node.get("required", []))
    for field in optional_fields.get(proto_name, set()):
        schema_field = camel(field)
        if schema_field in required:
            raise SystemExit(f"{schema_name}.{schema_field} is optional in proto but required in schema")
        prop = node.get("properties", {}).get(schema_field, {})
        variants = prop.get("anyOf", [])
        typ = prop.get("type")
        has_null_type = typ == "null" or (isinstance(typ, list) and "null" in typ)
        if not has_null_type and not any(variant.get("type") == "null" for variant in variants):
            raise SystemExit(f"{schema_name}.{schema_field} lacks nullable schema branch")


def enum_prefix(name):
    return re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", name).upper() + "_"


for enum_name, values in sorted(enum_values.items()):
    node = defs.get(enum_name)
    if node is None:
        raise SystemExit(f"schema missing definition for proto enum {enum_name}")
    prefix = enum_prefix(enum_name)
    proto_values = {
        value.removeprefix(prefix).lower().replace("_", "-")
        for value in values
        if not value.endswith("_UNSPECIFIED")
    }
    schema_values = set(node.get("enum", []))
    if proto_values != schema_values:
        raise SystemExit(
            f"{enum_name} enum drift: schema={sorted(schema_values)} proto={sorted(proto_values)}"
        )


terminal = defs.get("TerminalStatus", {})
variants = terminal.get("oneOf", [])
if len(variants) != 4:
    raise SystemExit("TerminalStatus schema does not expose exactly four oneOf variants")
for variant in variants:
    props = variant.get("properties", {})
    if props.get("outcome", {}).get("enum", [None])[0] not in {
        "exit-code",
        "signal",
        "status-code",
        "error",
    }:
        raise SystemExit("TerminalStatus variant has unexpected outcome discriminator")
PY

ok "guest-control-proto: guest_control.proto descriptor compiles and matches required shape"
