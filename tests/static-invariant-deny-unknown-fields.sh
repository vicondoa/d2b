#!/usr/bin/env bash
# Sensitive schemas reject unknown fields.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# Moved the live bundle baseline to schemas/v2; the deny-unknown
# fixtures track that current host.json shape.
SCHEMA_DIR=${SCHEMA_DIR:-$ROOT/docs/reference/schemas/v2}
FIXTURE_DIR=${FIXTURE_DIR:-$ROOT/tests/fixtures/deny-unknown}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

schemas=(privileges.json processes.json minijail-profile.json bundle.json host.json closures.json guest-control.json)
for schema in "${schemas[@]}"; do
  if [ ! -f "$SCHEMA_DIR/$schema" ]; then
    log "schemas absent — skipping static-invariant-deny-unknown-fields"
    exit 0
  fi
done

for stem in bundle host closures; do
  if [ ! -f "$FIXTURE_DIR/${stem}-valid.json" ] || [ ! -f "$FIXTURE_DIR/${stem}-invalid.json" ]; then
    fail "static-invariant-deny-unknown-fields: missing fixture pair for $stem under $FIXTURE_DIR"
  fi
done

python=$(cat <<'PY'
import copy
import json
import sys
from pathlib import Path
from jsonschema import Draft202012Validator

schema_path = Path(sys.argv[1])
valid_path = None if sys.argv[2] == "-" else Path(sys.argv[2])
invalid_path = None if sys.argv[3] == "-" else Path(sys.argv[3])

schema = json.loads(schema_path.read_text(encoding="utf-8"))
Draft202012Validator.check_schema(schema)
validator = Draft202012Validator(schema)
root = schema


def resolve(s):
    while isinstance(s, dict) and "$ref" in s:
        ref = s["$ref"]
        if not ref.startswith("#/"):
            raise SystemExit(f"external $ref not supported in {schema_path}: {ref}")
        node = root
        for part in ref[2:].split('/'):
            node = node[part.replace('~1', '/').replace('~0', '~')]
        s = node
    return s


def synth(s):
    s = resolve(s)
    if "const" in s:
        return s["const"]
    if "enum" in s and s["enum"]:
        return s["enum"][0]
    for key in ("oneOf", "anyOf", "allOf"):
        if key in s and s[key]:
            if key == "allOf":
                merged = {}
                for sub in s[key]:
                    val = synth(sub)
                    if isinstance(val, dict):
                        merged.update(val)
                    else:
                        return val
                return merged
            return synth(s[key][0])
    typ = s.get("type")
    if isinstance(typ, list):
        typ = next((t for t in typ if t != "null"), typ[0])
    if typ == "object" or "properties" in s:
        obj = {}
        props = s.get("properties", {})
        for name in s.get("required", []):
            obj[name] = synth(props.get(name, {"type": "string"}))
        return obj
    if typ == "integer":
        return int(s.get("minimum", 0) or 0)
    if typ == "number":
        return s.get("minimum", 0) or 0
    if typ == "boolean":
        return False
    if typ == "array":
        item = synth(s.get("items", {"type": "string"}))
        return [item] if int(s.get("minItems", 0) or 0) > 0 else []
    return "fixture"


def iter_object_schemas(s, path, seen):
    s = resolve(s)
    marker = id(s)
    if marker in seen:
        return
    seen.add(marker)

    if s.get("type") == "object" or "properties" in s:
        yield path, s

    for name, sub in s.get("definitions", {}).items():
        yield from iter_object_schemas(sub, f"{path}/definitions/{name}", seen)
    for name, sub in s.get("$defs", {}).items():
        yield from iter_object_schemas(sub, f"{path}/$defs/{name}", seen)
    for name, sub in s.get("properties", {}).items():
        yield from iter_object_schemas(sub, f"{path}/properties/{name}", seen)
    items = s.get("items")
    if isinstance(items, dict):
        yield from iter_object_schemas(items, f"{path}/items", seen)
    for key in ("oneOf", "anyOf", "allOf"):
        for idx, sub in enumerate(s.get(key, [])):
            yield from iter_object_schemas(sub, f"{path}/{key}/{idx}", seen)


def assert_nested_unknowns_rejected():
    if schema_path.name != "guest-control.json":
        return

    for path, sub_schema in iter_object_schemas(schema, "#", set()):
        instance = synth(sub_schema)
        if not isinstance(instance, dict):
            continue
        invalid = copy.deepcopy(instance)
        invalid["__nixling_unknown_field__"] = True
        subvalidator = validator.evolve(schema=sub_schema)
        errors = list(subvalidator.iter_errors(invalid))
        if not errors:
            print(f"unknown field accepted at {path}", file=sys.stderr)
            sys.exit(6)
        if not any(
            "Additional properties" in err.message or "Unevaluated properties" in err.message
            for err in errors
        ):
            for err in errors[:5]:
                print(f"unexpected nested validation error at {path}: {err.message}", file=sys.stderr)
            sys.exit(7)


def assert_guest_control_string_bounds():
    if schema_path.name != "guest-control.json":
        return

    expected = {
        "GuestSchemaVersion": 32,
        "GuestConnectRequestLine": 64,
        "GuestConnectAckLine": 64,
        "GuestVmId": 128,
        "RequestId": 128,
        "ExecId": 128,
        "GuestNonce": 128,
        "GuestBootId": 128,
        "CapabilitiesHash": 128,
        "GuestArg": 4096,
        "GuestUser": 128,
        "GuestCwd": 4096,
        "EnvKey": 128,
        "EnvValue": 8192,
    }

    definitions = schema.get("definitions", {})
    for name, max_length in expected.items():
        node = resolve(definitions.get(name, {}))
        if node.get("type") != "string":
            print(f"{name} is not emitted as a string schema", file=sys.stderr)
            sys.exit(8)
        if node.get("minLength") != 1 or node.get("maxLength") != max_length:
            print(
                f"{name} lost bounds: minLength={node.get('minLength')} maxLength={node.get('maxLength')}",
                file=sys.stderr,
            )
            sys.exit(9)


def assert_guest_control_chunk_bounds():
    if schema_path.name != "guest-control.json":
        return

    checks = {
        ("WriteStdinRequest", "data"): {"minItems": 1, "maxItems": 1048576},
        ("ReadOutputResponse", "data"): {"maxItems": 1048576},
        ("ExecLogsResponse", "data"): {"maxItems": 1048576},
        ("ReadOutputRequest", "maxLen"): {"minimum": 1.0, "maximum": 1048576.0},
        ("ExecLogsRequest", "maxLen"): {"minimum": 1.0, "maximum": 1048576.0},
    }

    for (definition, field), expected in checks.items():
        node = resolve(schema.get("definitions", {}).get(definition, {}))
        prop = resolve(node.get("properties", {}).get(field, {}))
        for key, value in expected.items():
            if prop.get(key) != value:
                print(
                    f"{definition}.{field} lost {key}: got {prop.get(key)!r}, want {value!r}",
                    file=sys.stderr,
                )
                sys.exit(10)


if valid_path is None:
    instance = synth(schema)
    if not isinstance(instance, dict):
        print("root schema did not synthesize to object", file=sys.stderr)
        sys.exit(3)
    invalid_instance = copy.deepcopy(instance)
    invalid_instance["__nixling_unknown_field__"] = True
else:
    instance = json.loads(valid_path.read_text(encoding="utf-8"))
    invalid_instance = json.loads(invalid_path.read_text(encoding="utf-8"))

valid_errors = list(validator.iter_errors(instance))
if valid_errors:
    for err in valid_errors[:5]:
        path = '/'.join(map(str, err.absolute_path)) or '<root>'
        print(f"valid fixture rejected at {path}: {err.message}", file=sys.stderr)
    sys.exit(2)

unknown_errors = list(validator.iter_errors(invalid_instance))
if not unknown_errors:
    print("unknown field was accepted", file=sys.stderr)
    sys.exit(4)

if not any("Additional properties" in err.message or "Unevaluated properties" in err.message for err in unknown_errors):
    for err in unknown_errors[:5]:
        print(f"unexpected validation error: {err.message}", file=sys.stderr)
    sys.exit(5)

assert_nested_unknowns_rejected()
assert_guest_control_string_bounds()
assert_guest_control_chunk_bounds()
PY
)

for schema in "${schemas[@]}"; do
  stem=${schema%.json}
  valid_fixture=$FIXTURE_DIR/${stem}-valid.json
  invalid_fixture=$FIXTURE_DIR/${stem}-invalid.json
  if [ ! -f "$valid_fixture" ] || [ ! -f "$invalid_fixture" ]; then
    valid_fixture=-
    invalid_fixture=-
  fi

  if nix-shell -p "python3.withPackages (ps: [ ps.jsonschema ])" --run "python3 - '$SCHEMA_DIR/$schema' '$valid_fixture' '$invalid_fixture' <<'PY'
$python
PY
" >/dev/null 2>&1; then
    ok "static-invariant-deny-unknown-fields: $schema accepts valid input and rejects unknown fields"
  else
    nix-shell -p "python3.withPackages (ps: [ ps.jsonschema ])" --run "python3 - '$SCHEMA_DIR/$schema' '$valid_fixture' '$invalid_fixture' <<'PY'
$python
PY
" 2>&1 | head -40 >&2 || true
    fail "static-invariant-deny-unknown-fields: $schema accepts unknown fields or rejects its valid fixture"
  fi
done
