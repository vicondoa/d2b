#!/usr/bin/env python3
import json
import pathlib
import sys

if len(sys.argv) != 3:
    raise SystemExit("usage: materialize-eval-fixture.py <input-json> <output-dir>")
source = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
with source.open(encoding="utf-8") as handle:
    fixture = json.load(handle)
out.mkdir(parents=True, exist_ok=True)
closures = out / "closures"
closures.mkdir(exist_ok=True)
for name, value in fixture["files"].items():
    path = out / name
    path.write_text(json.dumps(value, sort_keys=True, separators=(",", ":")), encoding="utf-8")
for vm, value in fixture["closures"].items():
    path = closures / f"{vm}.json"
    path.write_text(json.dumps(value, sort_keys=True, separators=(",", ":")), encoding="utf-8")
