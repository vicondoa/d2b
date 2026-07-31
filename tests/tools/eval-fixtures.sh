#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}
out=${1:?usage: eval-fixtures.sh <output-root>}

# shellcheck disable=SC1091
. "$ROOT/tests/lib.sh"
system=$(nix eval --raw --impure --expr builtins.currentSystem)
flake_ref=$(d2b_flake_ref "$ROOT")

mkdir -p "$out"
if [ "$system" != x86_64-linux ]; then
  printf '%s\n' "eval-fixtures: unavailable on $system (graphics fixture is x86_64-linux only)" >&2
  exit 3
fi
for kind in minimal full; do
  json="$out/$kind.json"
  nix eval --quiet --no-warn-dirty --json --apply \
    "fixtureFor: (fixtureFor \"${system}\").${kind}" "${flake_ref}#lib.evalFixture" > "$json"
  nix shell --quiet --inputs-from "$ROOT" nixpkgs#python3 --command python3 \
    "$ROOT/tests/tools/materialize-eval-fixture.py" "$json" "$out/$kind"
done
printf '%s\n%s\n' "$out/minimal" "$out/full"
