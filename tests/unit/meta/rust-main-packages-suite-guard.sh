#!/usr/bin/env bash
# tests/unit/meta/rust-main-packages-suite-guard.sh
#
# Layer-1 aggregate-suite membership guard (issue #584). `rust-main-packages`
# is the fixed suite that owns which per-package all-tests aggregates ride
# Layer-1. Each package that declares an all-tests aggregate must be listed
# in the suite, and its aggregate suite must not carry any positive tag that
# would exclude it from expansion by the Layer-1 parent suite.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}
CHECKS="$ROOT/bazel/checks/BUILD.bazel"

fail() {
    echo "rust-main-packages suite guard: $*" >&2
    exit 1
}

[ -f "$CHECKS" ] || fail "bazel/checks/BUILD.bazel not found (is ROOT set?)"

suite=$(sed -n '/test_suite(/,/^)/p' "$CHECKS" | sed -n '/name = "rust-main-packages"/,/^)/p')
suite_pkgs=$(printf '%s\n' "$suite" | grep -oE '//packages/[^:"]+' | sed 's#//packages/##' || true)

for build in "$ROOT"/packages/*/BUILD.bazel; do
    [ -f "$build" ] || continue
    pkg=$(basename "$(dirname "$build")")
    if ! grep -q 'name = "all-tests"' "$build"; then
        continue
    fi
    if ! printf '%s\n' "$suite_pkgs" | grep -qx "$pkg"; then
        fail "$pkg: all-tests suite present but absent from rust-main-packages"
    fi
    # No positive tag on the aggregate (only negative tags are allowed).
    block=$(sed -n '/test_suite(/,/^)/p' "$build" | sed -n '/name = "all-tests"/,/^)/p')
    tags=$(printf '%s\n' "$block" | sed -n 's/.*tags = \[\(.*\)\].*/\1/p')
    positive=$(printf '%s\n' "$tags" | tr ',' '\n' | sed 's/^ *//;s/ *$//' | grep -v '^-' | grep -v '^$' || true)
    if [ -n "$positive" ]; then
        fail "$pkg: all-tests aggregate carries positive tags: $positive"
    fi
done

echo "rust-main-packages suite membership ok"
