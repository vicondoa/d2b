#!/usr/bin/env bash
# tests/unit/meta/rust-main-packages-suite-guard.sh
#
# Layer-1 aggregate-suite membership guard (issue #584). `rust-main-packages`
# is the fixed suite that owns which per-package all-tests aggregates ride
# Layer-1. Each package that declares an all-tests aggregate must be listed
# in the suite, and its aggregate suite must not carry any positive tag that
# would exclude it from expansion by the Layer-1 parent suite.
#
# Deliberate non-members (intentional exclusions): packages that keep an
# all-tests aggregate on disk but are retired owners and must not ride
# Layer-1; the guard must not require them to appear in the suite. See
# AGENTS.md ("Retired ... realm-core owners are absent from the shared Cargo,
# copied-Guest, policy, and aggregate Bazel edges"). If a retired owner's
# BUILD.bazel is ever dropped or its aggregate removed, remove it here too.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}
# Overridable so negative probes can point the guard at a scratch tree.
CHECKS_FILE=${CHECKS_FILE:-"$ROOT/bazel/checks/BUILD.bazel"}
PKGS_ROOT=${PKGS_ROOT:-"$ROOT/packages"}

# Retired owners that keep an all-tests aggregate but are intentionally absent
# from rust-main-packages (AGENTS.md retirement rule).
EXCLUDED_PKGS=${EXCLUDED_PKGS:-"d2b-realm-core"}

fail() {
    echo "rust-main-packages suite guard: $*" >&2
    exit 1
}

[ -f "$CHECKS_FILE" ] || fail "bazel/checks/BUILD.bazel not found (is ROOT set?)"

# Membership extraction: drop comment lines first so prose that merely mentions
# a //packages/ target cannot masquerade as a suite entry.
suite=$(sed -n '/test_suite(/,/^)/p' "$CHECKS_FILE" | sed -n '/name = "rust-main-packages"/,/^)/p')
suite_pkgs=$(printf '%s\n' "$suite" | grep -v '^[[:space:]]*#' | grep -oE '//packages/[^:"]+' | sed 's#//packages/##' || true)

# Core assertion, factored out so negative probes exercise the same logic.
check_pkg() {
    local build=$1
    local pkg=$2

    [ -f "$build" ] || return 0
    if ! grep -q 'name = "all-tests"' "$build"; then
        return 0
    fi
    # Retired owners are exempt from the membership requirement.
    # EXCLUDED_PKGS is a space-separated list; word splitting is intended.
    # shellcheck disable=SC2086
    for excluded in $EXCLUDED_PKGS; do
        [ "$excluded" = "$pkg" ] && return 0
    done
    if ! printf '%s\n' "$suite_pkgs" | grep -qx "$pkg"; then
        fail "$pkg: all-tests aggregate present but absent from rust-main-packages"
    fi

    # No positive tag on the aggregate (only negative tags are allowed). Parse
    # the tags attribute across multiple lines, stripping comment text and
    # quotes before splitting, so a tag like "-no-remote-exec" is not mistaken
    # for a positive one (the quote around it used to survive the split). A
    # suite without a tags attribute must parse as empty, not as garbage.
    block=$(sed -n '/test_suite(/,/^)/p' "$build" | sed -n '/name = "all-tests"/,/^)/p')
    collapsed=$(printf '%s\n' "$block" | tr '\n' ' ' | sed 's/#.*$//')
    tags_attr=$(printf '%s' "$collapsed" | sed -n 's/.*tags = \[\([^]]*\)\].*/\1/p')
    clean=$(printf '%s\n' "$tags_attr" | sed 's/[,;]/\n/g' | sed 's/^[[:space:]]*//;s/[[:space:]]*$//' \
        | sed 's/^["'"'"']//;s/["'"'"']$//' | sed '/^$/d')
    positive=$(printf '%s\n' "$clean" | grep -v '^-' || true)
    if [ -n "$positive" ]; then
        fail "$pkg: all-tests aggregate carries positive tags: $(printf '%s\n' "$positive" | tr '\n' ' ')"
    fi
}

for build in "$PKGS_ROOT"/*/BUILD.bazel; do
    [ -f "$build" ] || continue
    check_pkg "$build" "$(basename "$(dirname "$build")")"
done

echo "rust-main-packages suite membership ok"
