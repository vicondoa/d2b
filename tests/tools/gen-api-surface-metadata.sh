#!/usr/bin/env bash
# Generate the exact rustdoc JSON census consumed by d2b-api-surface.
set -euo pipefail

ROOT=${ROOT:-$(cd "$(dirname "$(readlink -f "$0")")/../.." && pwd)}
public_dir=${1:?usage: gen-api-surface-metadata.sh <public-json-dir> <private-json-dir> <output>}
private_dir=${2:?usage: gen-api-surface-metadata.sh <public-json-dir> <private-json-dir> <output>}
output=${3:?usage: gen-api-surface-metadata.sh <public-json-dir> <private-json-dir> <output>}

nix shell --quiet --inputs-from "$ROOT" nixpkgs#python3 --command python3 - \
  "$public_dir" "$private_dir" "$output" <<'PY'
import json
import pathlib
import sys

public_dir = pathlib.Path(sys.argv[1])
private_dir = pathlib.Path(sys.argv[2])
output = pathlib.Path(sys.argv[3])


def load(path):
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def census(blob):
    return {
        "external_crates": len(blob["external_crates"]),
        "index_items": len(blob["index"]),
        "path_items": len(blob["paths"]),
    }


def hidden_count(blob):
    count = 0
    for item in blob["index"].values():
        for attribute in item.get("attrs", []):
            if "doc(hidden)" in json.dumps(attribute, separators=(",", ":")):
                count += 1
    return count


entries = []
public_files = sorted(public_dir.glob("*.json"))
if not public_files:
    raise SystemExit("api-surface metadata: public JSON census is empty")
for public_path in public_files:
    private_path = private_dir / public_path.name
    if not private_path.is_file():
        raise SystemExit("api-surface metadata: paired private JSON is missing")
    public = load(public_path)
    private = load(private_path)
    public_root = public["paths"][str(public["root"])]["path"]
    private_root = private["paths"][str(private["root"])]["path"]
    if len(public_root) != 1 or public_root != private_root:
        raise SystemExit("api-surface metadata: crate root mismatch")
    entries.append(
        {
            "crate_name": public_root[0],
            "private_census": census(private),
            "private_hidden_items": hidden_count(private),
            "private_json_file": public_path.name,
            "public_census": census(public),
            "public_json_file": public_path.name,
        }
    )

private_files = {path.name for path in private_dir.glob("*.json")}
expected = {entry["private_json_file"] for entry in entries}
if private_files != expected:
    raise SystemExit("api-surface metadata: private JSON file set differs")

target_triples = {
    blob["target"]["triple"]
    for path in [*public_files, *(private_dir / name for name in sorted(private_files))]
    for blob in [load(path)]
}
if len(target_triples) != 1:
    raise SystemExit("api-surface metadata: rustdoc target triples differ")

metadata = {
    "crates": entries,
    "nightly": "nightly-2026-02-16",
    "rustdoc_format_version": 57,
    "schema_version": 1,
    "target_triple": target_triples.pop(),
}
output.write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
