#!/usr/bin/env bash
# Every declared public/broker operation has a rendered privileges row.
#
# The SCHEMA path moved from v1 to v2 because `v2` is the current
# bundle baseline (bundleVersion=2, schemaVersion=v2). The privileges
# schema enum is regenerated from
# `packages/nixling-core/src/privileges.rs` by `cargo xtask gen-schemas`
# into `docs/reference/schemas/v2/privileges.json`; validating
# rendered operations against v1's frozen enum was correct under
# the baseline but drifts on every + operation addition.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
SCHEMA=${SCHEMA:-$ROOT/docs/reference/schemas/v2/privileges.json}
CLI_DOC=${CLI_DOC:-$ROOT/docs/reference/cli-contract.md}
BROKER_HINT=${BROKER_HINT:-$ROOT/packages/nixling-ipc/src/broker_wire.rs}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

cd "$ROOT"

if [ ! -f "$SCHEMA" ] || [ ! -f "$CLI_DOC" ] || [ ! -d "$ROOT/packages" ]; then
  log "privileges schema/doc/broker inputs absent — skipping privileges-matrix-completeness"
  exit 0
fi

if [ -z "${NIXLING_PRIVILEGES_MATRIX_IN_NIX_SHELL:-}" ] && ! command -v python3 >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "privileges-matrix-completeness: neither python3 nor nix is on PATH"
  fi
  log "  python3 not on PATH; re-entering via nix shell"
  export NIXLING_PRIVILEGES_MATRIX_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" nixpkgs#python3 --command bash "$0" "$@"
fi

scratch=$(nl_mktemp .privileges-matrix.XXXXXX)
rendered_json=$scratch/rendered-privileges.json
declared_ops_raw=$scratch/declared-ops.raw
declared_ops=$scratch/declared-ops.txt
rendered_ops=$scratch/rendered-ops.txt
missing=$scratch/missing.txt
add_cleanup "rm -rf -- \"$scratch\""

mapfile -t broker_sources < <(grep -rl --include='*.rs' 'DelegateCgroupV2' "$ROOT/packages" 2>/dev/null | sort)
if [ "${#broker_sources[@]}" -eq 0 ] && [ -f "$BROKER_HINT" ]; then
  broker_sources=("$BROKER_HINT")
fi

python3 - "$CLI_DOC" "${broker_sources[@]}" > "$declared_ops_raw" <<'PY'
import re
import sys
from pathlib import Path

found = set()


def record_cli(parts):
    if not parts:
        return
    if parts[0].startswith('NOTICE'):
        return
    if parts[0] in {'--help', '-h'}:
        return
    if re.fullmatch(r'v\d+(?:\.\d+)*', parts[0]):
        return
    if '/' in parts[0]:
        for item in parts[0].split('/'):
            record_cli([item] + parts[1:])
        return

    op = [parts[0]]
    if len(parts) > 1:
        if parts[0] == 'host' and not parts[1].startswith('--'):
            op.append(parts[1])
            if len(parts) > 2 and parts[2] in {'--apply', '--dry-run', '--read-only'}:
                op.append(parts[2])
        elif parts[0] == 'vm' and not parts[1].startswith('--'):
            # konsole is the `vm exec -it` wrapper; both route to the daemon
            # `exec` operation and share its admin-only authz row.
            vm_alias = {'start': 'up', 'stop': 'down', 'restart': 'restart', 'list': 'list', 'exec': 'exec', 'konsole': 'exec'}.get(parts[1])
            if vm_alias is not None:
                op = [vm_alias]
            else:
                op.append(parts[1])
        elif parts[0] in {'audio', 'auth', 'debug', 'keys', 'store', 'usb'} and not parts[1].startswith('--'):
            op.append(parts[1])
        elif (parts[0], parts[1]) in {('audit', '--human'), ('audit', '--json'), ('status', '--check-bridges')}:
            op.append(parts[1])

    found.add(' '.join(op))


cli = Path(sys.argv[1]).read_text(encoding='utf-8')
for raw_cmd in re.findall(r'`(nixling[^`]*)`', cli):
    if re.search(r'^nixling\s+audio\s+on\|off(?:\s|$)', raw_cmd):
        found.update({'audio on', 'audio off'})
        continue
    if re.search(r'^nixling\s+audio\s+mic\s+on\|off(?:\s|$)', raw_cmd):
        found.add('audio mic')
        continue
    if re.search(r'^nixling\s+audio\s+speaker\s+on\|off(?:\s|$)', raw_cmd):
        found.add('audio speaker')
        continue
    if raw_cmd.startswith('nixling audit') and '--human' in raw_cmd:
        found.add('audit --human')

    tokens = []
    for raw in raw_cmd.split()[1:]:
        token = raw.strip().rstrip('.,;:')
        if token.startswith('<') or token.startswith('[') or token.startswith('>'):
            continue
        token = token.split('|', 1)[0].rstrip(']')
        if token in {'...', '…'}:
            continue
        if token:
            tokens.append(token)
    record_cli(tokens)

for broker_path in map(Path, sys.argv[2:]):
    text = broker_path.read_text(encoding='utf-8')
    saw_enum = False
    # Match operation-style enums by name suffix. Capture the enum's name
    # so we can skip typed sub-error enums (CgroupOpError,
    # PidfdOpError, OpError, etc.) whose variants are kebab-case audit
    # error codes, NOT broker enum operations. The integrator fix excludes
    # enum-name suffixes Error/Err/Kind from the operation set.
    for enum_name, enum_body in re.findall(
        r'enum\s+(\w*(?:Operation|Request|Command|Op)\w*)\s*\{([^}]*)\}',
        text,
        flags=re.S,
    ):
        if enum_name.endswith(('Error', 'Err', 'Kind')):
            continue
        for variant in re.findall(r'^\s*([A-Z][A-Za-z0-9_]*)\b', enum_body, flags=re.M):
            found.add(variant)
            saw_enum = True
    if not saw_enum:
        for op in re.findall(r'row\(\s*"([A-Z][A-Za-z0-9]*)"', text):
            found.add(op)

for item in sorted(found):
    print(item)
PY

sort -u "$declared_ops_raw" > "$declared_ops"

if ! rendered_path=$(nl_smoke_bundle_privileges_json); then
  fail "privileges-matrix-completeness: could not render smoke privileges.json"
fi
cp "$rendered_path" "$rendered_json"

if ! nix-shell -p "python3.withPackages (ps: [ ps.jsonschema ])" --run "python3 - '$SCHEMA' '$rendered_json' '$rendered_ops' <<'PY'
import json
import sys
from pathlib import Path
from jsonschema import Draft202012Validator

schema = json.loads(Path(sys.argv[1]).read_text(encoding='utf-8'))
data = json.loads(Path(sys.argv[2]).read_text(encoding='utf-8'))
Draft202012Validator.check_schema(schema)
validator = Draft202012Validator(schema)
errors = list(validator.iter_errors(data))
if errors:
    for err in errors[:20]:
        path = '/'.join(map(str, err.absolute_path)) or '<root>'
        print(f'VALIDATION: {path} -> {err.message}', file=sys.stderr)
    sys.exit(1)
ops = sorted({row['operation'] for row in data.get('publicOperations', [])} | {row['operation'] for row in data.get('brokerOperations', [])})
Path(sys.argv[3]).write_text('\n'.join(ops) + ('\n' if ops else ''), encoding='utf-8')
PY
" >/dev/null 2>&1; then
  nix-shell -p "python3.withPackages (ps: [ ps.jsonschema ])" --run "python3 - '$SCHEMA' '$rendered_json' '$rendered_ops' <<'PY'
import json
import sys
from pathlib import Path
from jsonschema import Draft202012Validator

schema = json.loads(Path(sys.argv[1]).read_text(encoding='utf-8'))
data = json.loads(Path(sys.argv[2]).read_text(encoding='utf-8'))
validator = Draft202012Validator(schema)
for err in validator.iter_errors(data):
    path = '/'.join(map(str, err.absolute_path)) or '<root>'
    print(f'VALIDATION: {path} -> {err.message}', file=sys.stderr)
PY
" 2>&1 | head -80 >&2 || true
  fail "privileges-matrix-completeness: rendered privileges.json fails schema validation"
fi

if [ ! -s "$declared_ops" ]; then
  fail "privileges-matrix-completeness: no CLI/API or broker operations discovered"
fi
if [ ! -s "$rendered_ops" ]; then
  fail "privileges-matrix-completeness: rendered privileges.json contains no operations"
fi

declared_sorted=$scratch/declared-ops.csorted
rendered_sorted=$scratch/rendered-ops.csorted
LC_ALL=C sort -u "$declared_ops" > "$declared_sorted"
LC_ALL=C sort -u "$rendered_ops" > "$rendered_sorted"

LC_ALL=C comm -23 "$declared_sorted" "$rendered_sorted" > "$missing"
if [ -s "$missing" ]; then
  cat "$missing" >&2
  fail "privileges-matrix-completeness: declared operations missing from rendered privileges.json"
fi

ok "privileges-matrix-completeness: rendered privileges.json covers every declared CLI/API and broker operation"
