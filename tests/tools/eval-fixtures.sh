#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}
out=${1:?usage: eval-fixtures.sh <output-root>}
system=$(nix eval --raw --impure --expr builtins.currentSystem)

mkdir -p "$out"
if [ "$system" != x86_64-linux ]; then
  printf '%s\n' "eval-fixtures: unavailable on $system (graphics fixture is x86_64-linux only)" >&2
  exit 3
fi
for kind in minimal full; do
  json="$out/$kind.json"
  nix eval --quiet --no-warn-dirty --json --apply \
    "fixtureFor: (fixtureFor \"${system}\").${kind}" ".#lib.evalFixture" > "$json"
  nix shell --quiet --inputs-from "$ROOT" nixpkgs#python3 --command python3 \
    "$ROOT/tests/tools/materialize-eval-fixture.py" "$json" "$out/$kind"
done
printf '%s\n%s\n' "$out/minimal" "$out/full"
