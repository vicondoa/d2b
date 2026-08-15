#!/usr/bin/env bash
set -euo pipefail

# Standalone Cargo compatibility proof. This intentionally does not call
# Bazel: it protects the Cargo contract that remains useful before and after
# the Bazel migration.

ROOT=${ROOT:-$(cd "$(dirname "$(readlink -f "$0")")/../.." && pwd)}
cd "$ROOT"

if ! cargo nextest --version >/dev/null 2>&1; then
  if [ -z "${D2B_CARGO_COMPAT_NEXTEST_SHELL:-}" ] && command -v nix >/dev/null 2>&1; then
    export D2B_CARGO_COMPAT_NEXTEST_SHELL=1
    exec nix shell --quiet --inputs-from "$ROOT" nixpkgs#cargo-nextest \
      --command bash "$0" "$@"
  fi
  echo "cargo-compat: cargo-nextest is required" >&2
  exit 1
fi

echo "cargo-compat: root metadata and formatting"
cargo metadata --locked --offline --format-version 1 >/dev/null
cargo fmt --all --check

echo "cargo-compat: generic nextest excludes broker, guest runner, and fixtures"
cargo nextest list --locked --workspace \
  --exclude d2b-priv-broker \
  --exclude d2b-guest-shell-runner \
  --exclude d2b-contract-tests >/dev/null

echo "cargo-compat: serial broker feature contexts"
for feature in "" layer1-bootstrap fake-backends; do
  args=(--locked --package d2b-priv-broker --no-default-features)
  if [ -n "$feature" ]; then
    args+=(--features "$feature")
  fi
  cargo test "${args[@]}" --no-run
done

echo "cargo-compat: guest shell runner feature context"
cargo test --locked --package d2b-guest-shell-runner \
  --no-default-features --features real-libshpool --no-run

echo "cargo-compat: doctest surface"
cargo test --locked --package d2b-core --doc

metadata=$(cargo metadata --locked --offline --format-version 1)
mapfile -t harness_free_tests < <(
  printf '%s' "$metadata" | python3 -c '
import json
import pathlib
import sys
import tomllib

data = json.load(sys.stdin)
for package in data["packages"]:
    if package["id"] not in data["workspace_members"]:
        continue
    manifest = pathlib.Path(package["manifest_path"])
    with manifest.open("rb") as handle:
        manifest_data = tomllib.load(handle)
    for target in manifest_data.get("test", []):
        if target.get("harness") is False:
            print(
                package["name"],
                target["name"],
                ",".join(target.get("required-features", [])),
            )
'
)
if [ "${#harness_free_tests[@]}" -eq 0 ]; then
  echo "cargo-compat: no harness-free test targets discovered" >&2
  exit 1
fi
for row in "${harness_free_tests[@]}"; do
  read -r package target required_features <<<"$row"
  echo "cargo-compat: harness-free test $package/$target"
  args=(--locked --package "$package" --test "$target")
  if [ -n "$required_features" ]; then
    args+=(--features "$required_features")
  fi
  cargo test "${args[@]}"
done

mapfile -t benches < <(
  printf '%s' "$metadata" | python3 -c '
import json, sys
data = json.load(sys.stdin)
for package in data["packages"]:
    if package["id"] not in data["workspace_members"]:
        continue
    for target in package["targets"]:
        if "bench" in target["kind"]:
            print(package["name"], target["name"])
'
)
if [ "${#benches[@]}" -eq 0 ]; then
  echo "cargo-compat: no bench targets discovered" >&2
  exit 1
fi
for row in "${benches[@]}"; do
  read -r package target <<<"$row"
  echo "cargo-compat: bench $package/$target"
  cargo bench --locked --package "$package" --bench "$target" --no-run
done

echo "cargo-compat: fixture exclusion proof"
if cargo test --locked --workspace \
  --exclude d2b-priv-broker \
  --exclude d2b-guest-shell-runner \
  --exclude d2b-contract-tests --no-run >/dev/null; then
  echo "cargo-compat: fixture exclusion proof passed"
else
  echo "cargo-compat: fixture exclusion proof failed" >&2
  exit 1
fi

echo "cargo-compat: PASS"
