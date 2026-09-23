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
EXCLUDED_PKGS=${EXCLUDED_PKGS-"d2b-realm-core"}

fail() {
    echo "rust-main-packages suite guard: $*" >&2
    exit 1
}

[ -f "$CHECKS_FILE" ] || fail "bazel/checks/BUILD.bazel not found (is ROOT set?)"

# Membership extraction: drop comment lines first so prose that merely mentions
# a //packages/ target cannot masquerade as a suite entry. Capture the full
# //packages/<pkg>:<target> label and require the :all-tests target, so a
# suite entry aimed at a per-package unit/doctest suite cannot masquerade as
# membership either.
suite=$(sed -n '/test_suite(/,/^)/p' "$CHECKS_FILE" | sed -n '/name = "rust-main-packages"/,/^)/p')
suite_targets=$(printf '%s\n' "$suite" | grep -v '^[[:space:]]*#' | grep -oE '//packages/[^:"]+:[^"]*' || true)
bad_targets=$(printf '%s\n' "$suite_targets" | grep -v ':all-tests$' || true)
if [ -n "$bad_targets" ]; then
    fail "rust-main-packages entries must target :all-tests: $(printf '%s\n' "$bad_targets" | tr '\n' ' ')"
fi
suite_pkgs=$(printf '%s\n' "$suite_targets" | sed 's#//packages/##; s/:all-tests$//' | sort -u)
suite_label_count=$(printf '%s\n' "$suite_pkgs" | grep -c . || true)

# Intentional exclusions must stay real: a retired owner that loses its
# BUILD.bazel or its all-tests aggregate would otherwise be silently exempt.
# EXCLUDED_PKGS is a space-separated list; word splitting is intended.
# shellcheck disable=SC2086
excluded_count=0
for excluded in $EXCLUDED_PKGS; do
    excluded_count=$((excluded_count + 1))
    excluded_build="$PKGS_ROOT/$excluded/BUILD.bazel"
    if [ ! -f "$excluded_build" ]; then
        fail "excluded package $excluded: BUILD.bazel missing (remove it from EXCLUDED_PKGS?)"
    fi
    if ! grep -q 'name = "all-tests"' "$excluded_build"; then
        fail "excluded package $excluded: all-tests aggregate missing (remove it from EXCLUDED_PKGS?)"
    fi
done

# Core assertion, factored out so negative probes exercise the same logic.
checked=0
check_pkg() {
    local build=$1
    local pkg=$2

    [ -f "$build" ] || return 0
    if ! grep -q 'name = "all-tests"' "$build"; then
        return 0
    fi
    checked=$((checked + 1))
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
    # Anchor the block on the test_suite( line (not on the name line), so
    # attributes written before `name` stay visible, and strip each line's
    # comment tail BEFORE collapsing newlines, so a comment between `name`
    # and `tags` cannot delete the rest of the block.
    block=$(awk '
        /test_suite\(/ { buf = $0; in_block = 1; next }
        in_block { buf = buf "\n" $0 }
        /^\)/ { if (in_block) { if (buf ~ /name = "all-tests"/) print buf; in_block = 0 } }
    ' "$build")
    collapsed=$(printf '%s\n' "$block" | sed 's/#.*$//' | tr '\n' ' ')
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

# The corpus must not silently shrink: an empty/missing PKGS_ROOT, or a
# package BUILD file dropped with its suite label still present, would
# otherwise exit 0 with nothing (or fewer) checks run.
expected=$((suite_label_count + excluded_count))
if [ "$checked" -eq 0 ]; then
    fail "no package BUILD files checked under $PKGS_ROOT (empty or missing corpus?)"
fi
if [ "$checked" -ne "$expected" ]; then
    fail "checked $checked all-tests aggregates but the suite names $expected ($suite_label_count labels + $excluded_count exclusions)"
fi

echo "rust-main-packages suite membership ok"
