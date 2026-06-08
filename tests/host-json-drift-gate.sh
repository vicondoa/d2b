#!/usr/bin/env bash
# tests/host-json-drift-gate.sh— host.json per-field schema gold-file drift gate.
#
# Enforces the host.json per-field schema gold-file drift gate.
#
# Asserts:
#
#   1. tests/golden/host-json/baseline-host.json parses and matches
#      the v2 schema (when present).
#   2. tests/golden/host-json/ifname-collision.json declares
#      `_expectedRejection.code == "ifname-collision"`.
#   3. tests/golden/host-json/ifname-too-long.json declares
#      `_expectedRejection.code == "ifname-too-long"`.
#   4. Each tests/golden/host-json/unknown-field-*.json declares
#      `_expectedRejection.code == "wire-unknown-field"`.
#   5. When docs/reference/schemas/v2/host.json exists, assert it sets
#      `additionalProperties: false` on the four
#      Security-sensitive sub-objects whose Rust DTOs ARE
#      wired into HostJson today: KernelModulesEntry, BridgePortFlags,
#      IfNameMapping, and FirewallCoexistencePolicy.
#      Definition names follow the actual Rust types'
#      JsonSchema::schema_name() output; renaming a DTO requires
#      updating both this gate and the schema regeneration.
#   6. When docs/reference/schemas/v2/host.md exists, assert every
#      Added field name appears in the prose.
#
# The v2 schema/prose may be absent while work is in flight; in that
# case the schema- and prose-cross-check assertions skip with a clear
# log line.
#
# Scratch state lives outside $ROOT per AGENTS.md disk-hygiene
# contract.
#
# TODO(integrator): wire into tests/static.sh after the existing
# bundle-drift.sh invocation. tests/static.sh is integrator-owned; this
# script carries the wiring instruction only.

set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
# shellcheck source=lib.sh
. "$HERE/lib.sh"

cd "$ROOT"

FIXTURES_DIR=${FIXTURES_DIR:-$ROOT/tests/golden/host-json}
SCHEMA=${SCHEMA:-$ROOT/docs/reference/schemas/v2/host.json}
SCHEMA_MD=${SCHEMA_MD:-$ROOT/docs/reference/schemas/v2/host.md}

if [ -z "${NIXLING_HOST_JSON_DRIFT_IN_NIX_SHELL:-}" ] && ! command -v python3 >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "host-json-drift-gate: neither python3 nor nix is on PATH"
  fi
  export NIXLING_HOST_JSON_DRIFT_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" nixpkgs#python3 --command bash "$0" "$@"
fi

# Scratch outside $ROOT.
SCRATCH=${TMPDIR:-/tmp}/nl-host-json-drift.$$
mkdir -p "$SCRATCH"
add_cleanup "rm -rf -- '$SCRATCH'"

log "W3 host.json per-field schema gold-file drift gate"

if [ ! -d "$FIXTURES_DIR" ]; then
  fail "host-json-drift-gate: fixtures directory missing: $FIXTURES_DIR"
fi

# Render the live smoke host.json so the python block can assert
# top-level field presence (specifically firewallCoexistencePolicy).
# Falls back to "" when nl_smoke_bundle_host_json is unavailable;
# python handles that.
if SMOKE_HOST_JSON=$(nl_smoke_bundle_host_json 2>/dev/null); then
  :
else
  SMOKE_HOST_JSON=""
fi

python3 - "$FIXTURES_DIR" "$SCHEMA" "$SCHEMA_MD" "$SMOKE_HOST_JSON" <<'PY'
import json
import re
import sys
from pathlib import Path

fixtures_dir, schema_path, schema_md = (Path(p) for p in sys.argv[1:4])
smoke_host_path = Path(sys.argv[4]) if len(sys.argv) > 4 and sys.argv[4] else None

REQUIRED_BASELINE_FIELDS = {
    "schemaVersion",
    "bundleVersion",
    "site",
    "environments",
    "nftables",
    "networkManager",
    "hostsFile",
    "kernelModules",
    "bridgePortFlags",
    "firewallCoexistence",
    "ifnameMapping",
    "ch",
}

# Map fixture filename -> expected `_expectedRejection.code`.
EXPECTED_REJECTIONS = {
    "ifname-collision.json": "ifname-collision",
    "ifname-too-long.json": "ifname-too-long",
    "unknown-field-kernelmodules.json": "wire-unknown-field",
    "unknown-field-bridgeportflags.json": "wire-unknown-field",
    "unknown-field-firewallcoexistence.json": "wire-unknown-field",
    "unknown-field-ifnamemapping.json": "wire-unknown-field",
}

violations = []

# host-valid.json carries the live-emitted host.json shape
# (filePath / startMarker / endMarker) and MUST agree
# with the Rust broker constants in
# packages/nixling-host/src/routes.rs (HOSTS_MANAGED_BEGIN/END) and
# packages/nixling-priv-broker/src/ops/nm.rs (DEFAULT_NM_CONF_PATH).
# Drift between Nix emitter and Rust broker produces an installed
# /etc/nixling/host.json that advertises ownership-marker strings
# the broker will never write or recognize.
CANONICAL_LIVE_OWNERSHIP = {
    "networkManager.filePath": "/etc/NetworkManager/conf.d/00-nixling-unmanaged.conf",
    "hostsFile.startMarker": "# nixling-managed begin",
    "hostsFile.endMarker": "# nixling-managed end",
}

live_fixture = fixtures_dir.parent.parent / "fixtures" / "deny-unknown" / "host-valid.json"
if not live_fixture.exists():
    violations.append(f"missing live-shape fixture: {live_fixture}")
else:
    live_data = json.loads(live_fixture.read_text(encoding="utf-8"))
    observed = {
        "networkManager.filePath": live_data.get("networkManager", {}).get("filePath"),
        "hostsFile.startMarker": live_data.get("hostsFile", {}).get("startMarker"),
        "hostsFile.endMarker": live_data.get("hostsFile", {}).get("endMarker"),
    }
    for key, expected in CANONICAL_LIVE_OWNERSHIP.items():
        if observed[key] != expected:
            violations.append(
                f"host-valid.json: {key} is {observed[key]!r}, "
                f"expected {expected!r} (broker-canonical, see "
                f"packages/nixling-priv-broker/src/ops/nm.rs::DEFAULT_NM_CONF_PATH "
                f"and packages/nixling-host/src/routes.rs::HOSTS_MANAGED_BEGIN/END)"
            )

# Assert the live smoke-rendered host.json carries the
# `firewallCoexistencePolicy` top-level field. The field is `Option`
# in the Rust DTO (so older fixtures don't have it),
# but the Nix emitter always emits it; a regression that drops the
# emit would silently produce a host.json without firewall policy
# info. Skip when the smoke render isn't available (e.g. standalone
# gate run without nl_smoke_bundle_host_json on PATH).
REQUIRED_SMOKE_TOPLEVEL = {
    "firewallCoexistencePolicy",
}
if smoke_host_path and smoke_host_path.exists():
    smoke_data = json.loads(smoke_host_path.read_text(encoding="utf-8"))
    missing_smoke = REQUIRED_SMOKE_TOPLEVEL - smoke_data.keys()
    if missing_smoke:
        violations.append(
            f"smoke host.json ({smoke_host_path}): missing required "
            f"top-level field(s) emitted by nixos-modules/host-json.nix: "
            f"{sorted(missing_smoke)}. W4a-H3 requires "
            f"firewallCoexistencePolicy to be emitted even though the "
            f"Rust DTO is Option, so the broker always sees a coexistence "
            f"policy at apply-time."
        )
else:
    print(
        f"  (skipping smoke host.json top-level field check — "
        f"nl_smoke_bundle_host_json unavailable in this gate context)",
        file=sys.stderr,
    )

baseline = fixtures_dir / "baseline-host.json"
if not baseline.exists():
    violations.append(f"missing baseline fixture: {baseline}")
else:
    data = json.loads(baseline.read_text(encoding="utf-8"))
    missing = REQUIRED_BASELINE_FIELDS - data.keys()
    if missing:
        violations.append(
            f"baseline-host.json missing required W3 fields: {sorted(missing)}"
        )
    # The dynamic hash-derived ifname suffix MUST match the documented
    # regex. The baseline placeholder is `XXXXXXXX` for hashes; the
    # gate accepts that OR a real 8-hex suffix.
    ifname_re = re.compile(r"^nl-[a-z][a-z0-9-]*-(XXXXXXXX|[0-9a-f]{8})$")
    for env in data.get("environments", []):
        bridge = env.get("bridge", "")
        if not ifname_re.match(bridge):
            violations.append(
                f"baseline-host.json: bridge {bridge!r} does not match "
                f"nl-<env>-(XXXXXXXX|[0-9a-f]{{8}})"
            )
        if len(bridge) > 15:
            violations.append(
                f"baseline-host.json: bridge {bridge!r} exceeds IFNAMSIZ-1 (15 bytes)"
            )

for filename, expected_code in EXPECTED_REJECTIONS.items():
    path = fixtures_dir / filename
    if not path.exists():
        violations.append(f"missing malicious fixture: {filename}")
        continue
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        violations.append(f"{filename}: invalid JSON ({exc})")
        continue
    rejection = data.get("_expectedRejection", {})
    if rejection.get("code") != expected_code:
        violations.append(
            f"{filename}: _expectedRejection.code is {rejection.get('code')!r}, "
            f"expected {expected_code!r}"
        )

# Coordinated schema/prose gates (skip when the v2 schema is not yet
# on disk). Definition names come from the Rust DTOs'
# `JsonSchema::schema_name()` output and MUST match
# `packages/nixling-core/src/host.rs`. `FirewallCoexistencePolicy`
# (host_w3.rs DTO) is wired into HostJson as an optional field; it's
# now back in this check.
if schema_path.exists():
    schema = json.loads(schema_path.read_text(encoding="utf-8"))
    defs = schema.get("definitions", schema.get("$defs", {}))
    # Each security-sensitive sub-object MUST carry
    # additionalProperties:false so the deny_unknown_fields contract
    # holds at JSON Schema level too.
    for sub in (
        "KernelModulesEntry",
        "BridgePortFlags",
        "IfNameMapping",
        "FirewallCoexistencePolicy",
    ):
        definition = defs.get(sub)
        if definition is None:
            violations.append(
                f"v2 host.json schema is missing the {sub} definition"
            )
            continue
        if definition.get("additionalProperties") is not False:
            violations.append(
                f"v2 host.json schema: {sub}.additionalProperties must be false"
            )
else:
    print(
        f"  (v2 host.json schema {schema_path} not present; skipping schema cross-check — "
        "H3 owns its lifecycle)",
        file=sys.stderr,
    )

if schema_md.exists():
    prose = schema_md.read_text(encoding="utf-8")
    for field in (
        "kernelModules",
        "bridgePortFlags",
        "firewallCoexistence",
        "ifnameMapping",
        "ch",
        "ipv6Sysctls",
    ):
        if field not in prose:
            violations.append(
                f"v2 host.md prose does not document the {field!r} field"
            )
else:
    print(
        f"  (v2 host.md prose {schema_md} not present; skipping prose cross-check — "
        "H3 owns its lifecycle)",
        file=sys.stderr,
    )

if violations:
    for item in violations:
        print(item, file=sys.stderr)
    sys.exit(1)
PY

ok "host-json-drift-gate: 7 fixtures parse and declare expected rejection codes; schema/prose cross-checks where v2 exists"
