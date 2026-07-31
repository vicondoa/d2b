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
def safe_component(value: str, label: str) -> str:
    path = pathlib.PurePosixPath(value)
    if value in {"", ".", ".."} or path.is_absolute() or len(path.parts) != 1:
        raise SystemExit(f"unsafe {label}")
    return value


out.mkdir(parents=True, exist_ok=True)
closures = out / "closures"
closures.mkdir(exist_ok=True)
for name, value in fixture["files"].items():
    path = out / safe_component(name, "fixture filename")
    path.write_text(json.dumps(value, sort_keys=True, separators=(",", ":")), encoding="utf-8")
for vm, value in fixture["closures"].items():
    path = closures / f"{safe_component(vm, 'closure name')}.json"
    path.write_text(json.dumps(value, sort_keys=True, separators=(",", ":")), encoding="utf-8")
