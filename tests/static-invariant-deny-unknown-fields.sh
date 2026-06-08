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

schemas=(privileges.json processes.json minijail-profile.json bundle.json host.json closures.json)
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
    if typ == "array":
        return []
    if typ == "integer":
        return 0
    if typ == "number":
        return 0
    if typ == "boolean":
        return False
    return "fixture"


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
