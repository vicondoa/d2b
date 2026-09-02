#!/usr/bin/env bash
# tests/unit/meta/w0-dep-direction.sh - ADR 0032 crate-granular dependency
# direction + lint-inheritance gate.
#
# This is a workspace/lock integrity policy. It reads the manifests that are
# rules_rs metadata authority directly with Python's TOML parser, so the gate
# never needs a second build system or a generated dependency inventory.

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}

exec python3 - "$ROOT" <<'PY'
from __future__ import annotations

import pathlib
import sys
import tomllib


root = pathlib.Path(sys.argv[1])
packages = root / "packages"

try:
    with (root / "Cargo.toml").open("rb") as handle:
        workspace = tomllib.load(handle)["workspace"]
except (OSError, tomllib.TOMLDecodeError, KeyError) as error:
    print(f"FAIL: cannot parse workspace manifest: {error}", file=sys.stderr)
    raise SystemExit(1)

for lock_path in (root / "Cargo.lock", root / "packages" / "Cargo.guest.lock"):
    try:
        with lock_path.open("rb") as handle:
            lock = tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as error:
        print(f"FAIL: cannot parse lockfile {lock_path}: {error}", file=sys.stderr)
        raise SystemExit(1)
    if not isinstance(lock.get("package"), list) or not lock["package"]:
        print(f"FAIL: lockfile {lock_path} has no package records", file=sys.stderr)
        raise SystemExit(1)


def manifest_for(member: str) -> tuple[str, dict]:
    path = root / member / "Cargo.toml"
    try:
        with path.open("rb") as handle:
            manifest = tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as error:
        print(f"FAIL: cannot parse {path}: {error}", file=sys.stderr)
        raise SystemExit(1)
    name = manifest.get("package", {}).get("name")
    if not isinstance(name, str):
        print(f"FAIL: {path} has no package.name", file=sys.stderr)
        raise SystemExit(1)
    return name, manifest


def dependency_names(manifest: dict) -> set[str]:
    names: set[str] = set()
    tables = [manifest.get("dependencies", {}), manifest.get("build-dependencies", {})]
    for key, value in manifest.items():
        if key.startswith("target.") and isinstance(value, dict):
            tables.extend([value.get("dependencies", {}), value.get("build-dependencies", {})])
    for table in tables:
        for name, spec in table.items():
            if isinstance(spec, dict):
                names.add(spec.get("package", name))
            else:
                names.add(name)
    return names


def check_lints(crate: str, manifest: dict) -> bool:
    lints = manifest.get("lints", {})
    if not isinstance(lints, dict) or lints.get("workspace") is not True:
        print(f"FAIL: {crate}: missing [lints] workspace = true", file=sys.stderr)
        return False
    return True


allowed = {
    "d2b-contracts": set(),
}
members: dict[str, tuple[str, dict]] = {}
workspace_members = set(workspace.get("members", []))
for crate in allowed:
    member = f"packages/{crate}"
    if member not in workspace_members:
        continue
    name, manifest = manifest_for(member)
    members[name] = (member, manifest)

for crate, permitted in allowed.items():
    if crate not in members:
        print(f"  skip {crate} (not a workspace member)", file=sys.stderr)
        continue
    member, manifest = members[crate]
    print(f"  checking {crate} (manifest metadata)", file=sys.stderr)
    ok = check_lints(crate, manifest)
    for dependency in sorted(dependency_names(manifest)):
        if dependency in permitted:
            continue
        if dependency.startswith("d2b") or dependency in {"prost", "prost-types"}:
            print(
                f"FAIL: {crate} declares forbidden dependency {dependency!r} "
                "(dependency-direction violation)",
                file=sys.stderr,
            )
            ok = False
        elif dependency in members:
            print(
                f"FAIL: {crate} declares forbidden workspace dependency {dependency!r} "
                "(dependency-direction violation)",
                file=sys.stderr,
            )
            ok = False
    if not ok:
        raise SystemExit(1)

print("w0-dep-direction OK", file=sys.stderr)
PY
