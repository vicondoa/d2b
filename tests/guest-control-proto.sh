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
  Hello Authenticate Capabilities Health ExecCreate ExecInspect ExecWait ExecLogs \
  WriteStdin ReadOutput CloseStdin TtyWinResize ExecSignal ExecCancel
do
  require_descriptor "name: \"$method\"" "method $method"
done

for field in \
  guest_boot_id pending_read_output_waits_per_stream pending_exec_waits_per_vm \
  rpc_rate_per_connection_per_second rpc_rate_per_vm_burst end_offset \
  timed_out retry_after_ms host_auth_tag guest_auth_tag
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
for name, body in blocks("message"):
    fields = []
    for line in body.splitlines():
        line = line.strip()
        match = re.match(r"(optional\s+)?(repeated\s+)?([.\w]+)\s+(\w+)\s*=\s*(\d+)\s*;", line)
        if match:
            field = {
                "name": match.group(4),
                "type": match.group(3),
                "optional": bool(match.group(1)),
                "repeated": bool(match.group(2)),
                "number": int(match.group(5)),
            }
            fields.append(field)
    message_fields[name] = fields


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

scalar_types = {
    "string": ("string", None),
    "bool": ("boolean", None),
    "uint32": ("integer", "uint32"),
    "uint64": ("integer", "uint64"),
    "int32": ("integer", "int32"),
}


def schema_core(prop):
    if "anyOf" in prop:
        variants = [variant for variant in prop["anyOf"] if variant.get("type") != "null"]
        if len(variants) == 1:
            return variants[0]
    typ = prop.get("type")
    if isinstance(typ, list):
        non_null = [entry for entry in typ if entry != "null"]
        if len(non_null) == 1:
            copy = dict(prop)
            copy["type"] = non_null[0]
            return copy
    return prop


def ref_name(prop):
    ref = prop.get("$ref")
    if ref and ref.startswith("#/definitions/"):
        return ref.rsplit("/", 1)[-1]
    return None


def deref(prop):
    referenced = ref_name(prop)
    if referenced:
        return defs.get(referenced, {})
    return prop


def assert_scalar_shape(proto_name, field, prop):
    core = schema_core(prop)
    proto_type = field["type"]

    if field["repeated"]:
        if core.get("type") != "array":
            raise SystemExit(f"{proto_name}.{field['name']} is repeated in proto but not array in schema")
        item = schema_core(core.get("items", {}))
        if proto_type == "bytes":
            raise SystemExit(f"{proto_name}.{field['name']} cannot be repeated bytes")
        assert_scalar_shape(proto_name, {**field, "repeated": False}, item)
        return

    if proto_type == "bytes":
        core = deref(core)
        item = schema_core(core.get("items", {}))
        if (
            core.get("type") != "array"
            or item.get("type") != "integer"
            or item.get("format") != "uint8"
            or item.get("minimum") != 0.0
            or item.get("maximum") != 255.0
        ):
            raise SystemExit(f"{proto_name}.{field['name']} bytes field is not byte-array shaped in schema")
        return

    if proto_type in scalar_types:
        expected_type, expected_format = scalar_types[proto_type]
        if core.get("type") == expected_type and (
            expected_format is None or core.get("format") == expected_format
        ):
            return
        if proto_type == "string":
            referenced = ref_name(core)
            if referenced and schema_core(defs.get(referenced, {})).get("type") == "string":
                return
        raise SystemExit(
            f"{proto_name}.{field['name']} type drift: proto {proto_type}, schema {core}"
        )

    schema_ref = ref_name(core)
    expected_ref = message_map.get(proto_type, proto_type)
    if schema_ref != expected_ref:
        raise SystemExit(
            f"{proto_name}.{field['name']} ref drift: proto {proto_type}, schema {schema_ref}"
        )


skip_messages = {"TerminalStatus"}
hello_response_fields = {field["name"] for field in message_fields.get("HelloResponse", [])}
if "health" in hello_response_fields or "capabilities_hash" in hello_response_fields:
    raise SystemExit("HelloResponse must not expose pre-auth health or capabilities_hash")
if "AuthenticateRequest" not in message_fields or "AuthenticateResponse" not in message_fields:
    raise SystemExit("Authenticate messages missing from proto")
hello_response_body = dict(blocks("message")).get("HelloResponse", "")
if 'reserved 4, 5;' not in hello_response_body or 'reserved "capabilities_hash", "health";' not in hello_response_body:
    raise SystemExit("HelloResponse must reserve retired pre-auth health/capability fields")


def assert_field(proto_name, field_name, proto_type, number, optional=False):
    fields = {
        field["name"]: field
        for field in message_fields.get(proto_name, [])
    }
    field = fields.get(field_name)
    if field is None:
        raise SystemExit(f"{proto_name}.{field_name} missing")
    if field["type"] != proto_type or field["number"] != number or field["optional"] != optional:
        raise SystemExit(
            f"{proto_name}.{field_name} drifted: got type={field['type']} number={field['number']} optional={field['optional']}"
        )


assert_field("HelloRequest", "host_nonce", "bytes", 2)
assert_field("HelloResponse", "guest_nonce", "bytes", 1)
assert_field("AuthenticateRequest", "host_auth_tag", "bytes", 6)
assert_field("AuthenticateResponse", "guest_auth_tag", "bytes", 1, optional=True)
assert_field("AuthenticateResponse", "capabilities_hash", "string", 2, optional=True)
assert_field("AuthenticateResponse", "health", "HealthResponse", 3)
assert_field("AuthenticateResponse", "capabilities", "CapabilitiesResponse", 4)

limit_props = defs.get("GuestEffectiveLimits", {}).get("properties", {})
limit_maxima = {
    "maxChunkBytes": 1048576,
    "maxRecvMessageBytes": 4194304,
    "decodedWriteStdinBytesPerConnection": 16777216,
    "writeStdinHandlersPerConnection": 4,
    "stdinQueueChunksPerExec": 1,
    "stdoutLiveBufferBytes": 8388608,
    "stderrLiveBufferBytes": 8388608,
    "detachedStdoutLogBytes": 134217728,
    "detachedStderrLogBytes": 134217728,
    "longPollTimeoutMs": 1000,
    "slowConsumerGraceMs": 300000,
    "execSessionsPerVm": 256,
    "attachedSessionsPerVm": 64,
    "pendingReadOutputWaitsPerStream": 512,
    "pendingExecWaitsPerVm": 512,
    "rpcRatePerConnectionPerSecond": 200,
    "rpcRatePerVmBurst": 1000,
}
for field_name, expected_maximum in limit_maxima.items():
    prop = schema_core(limit_props.get(field_name, {}))
    if prop.get("maximum") != float(expected_maximum):
        raise SystemExit(f"GuestEffectiveLimits.{field_name} missing maximum {expected_maximum}")
for field_name in ("maxChunkBytes", "maxRecvMessageBytes"):
    prop = schema_core(limit_props.get(field_name, {}))
    if prop.get("minimum") != 1.0:
        raise SystemExit(f"GuestEffectiveLimits.{field_name} must have minimum 1")
for proto_name, fields in sorted(message_fields.items()):
    if proto_name in skip_messages:
        continue
    schema_name = message_map.get(proto_name, proto_name)
    node = defs.get(schema_name)
    if node is None:
        raise SystemExit(f"schema missing definition for proto message {proto_name}")
    props = set(node.get("properties", {}).keys())
    wanted = {camel(field["name"]) for field in fields}
    if props != wanted:
        raise SystemExit(
            f"{proto_name}/{schema_name} field drift: schema={sorted(props)} proto={sorted(wanted)}"
        )
    required = set(node.get("required", []))
    for field in fields:
        schema_field = camel(field["name"])
        prop = node.get("properties", {}).get(schema_field, {})
        assert_scalar_shape(proto_name, field, prop)
        if field["optional"]:
            if schema_field in required:
                raise SystemExit(f"{schema_name}.{schema_field} is optional in proto but required in schema")
            variants = prop.get("anyOf", [])
            typ = prop.get("type")
            has_null_type = typ == "null" or (isinstance(typ, list) and "null" in typ)
            if not has_null_type and not any(variant.get("type") == "null" for variant in variants):
                raise SystemExit(f"{schema_name}.{schema_field} lacks nullable schema branch")
        elif not field["repeated"] and field["type"] in scalar_types and schema_field not in required:
            raise SystemExit(f"{schema_name}.{schema_field} is non-optional scalar in proto but optional in schema")


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
expected_terminal = {
    "exit-code": {"name": "exit_code", "type": "int32", "optional": False, "repeated": False},
    "signal": {"name": "signal", "type": "uint32", "optional": False, "repeated": False},
    "status-code": {"name": "status_code", "type": "int32", "optional": False, "repeated": False},
    "error": {"name": "error", "type": "GuestControlErrorKind", "optional": False, "repeated": False},
}
for variant in variants:
    props = variant.get("properties", {})
    outcome = props.get("outcome", {}).get("enum", [None])[0]
    if outcome not in expected_terminal:
        raise SystemExit("TerminalStatus variant has unexpected outcome discriminator")
    payload = expected_terminal[outcome]
    payload_field = camel(payload["name"])
    if set(props) != {"outcome", payload_field}:
        raise SystemExit(f"TerminalStatus {outcome} variant payload drift: {sorted(props)}")
    assert_scalar_shape("TerminalStatus", payload, props[payload_field])
PY

ok "guest-control-proto: guest_control.proto descriptor compiles and matches required shape"
