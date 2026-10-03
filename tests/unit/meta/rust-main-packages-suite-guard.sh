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
# (d2b-realm-core was retired this way: its crate and BUILD.bazel are gone,
# so the exclusion list is empty by default today.)
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# Which tree this guard reads decides whether its answer means anything, and
# this has been the whole bug twice now.
#
# It used to read the live working tree, because the runfiles tree was an
# incomplete MATERIALIZED snapshot. It was incomplete for a concrete,
# fixable reason - the target's data declared each package's Cargo.toml and
# sources but not its BUILD.bazel - so the target now declares every
# packages/*/BUILD.bazel (see tests/unit/meta/BUILD.bazel). The staged tree
# is therefore complete, and it is a better answer than the live tree for
# two reasons: Bazel resolves the file list at analysis time, so it cannot
# be half-staged; and nothing else in the build can mutate it mid-scan,
# whereas the live tree is shared with every concurrently-running action.
#
# So under Bazel read the runfiles tree, which is what the target stages for
# this test and the only tree that is guaranteed complete and immutable for
# the duration of the run. A standalone run (no TEST_SRCDIR) still resolves
# the repository from the script's own location.
if [ -n "${TEST_SRCDIR:-}" ] && [ -n "${TEST_WORKSPACE:-}" ] \
    && [ -f "${TEST_SRCDIR}/${TEST_WORKSPACE}/bazel/checks/BUILD.bazel" ]; then
    ROOT=${TEST_SRCDIR}/${TEST_WORKSPACE}
else
    ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}
fi
# Overridable so negative probes can point the guard at a scratch tree.
CHECKS_FILE=${CHECKS_FILE:-"$ROOT/bazel/checks/BUILD.bazel"}
PKGS_ROOT=${PKGS_ROOT:-"$ROOT/packages"}

fail() {
    echo "rust-main-packages suite guard: $*" >&2
    exit 1
}

# The membership question is a comparison between the suite labels and the
# packages on disk, and it is answered by a sequence of reads: a glob, then
# an awk per package, then a count. If the tree can change between those
# reads, the answer depends on WHEN each file was read rather than on what
# the tree contains - a package whose BUILD file vanishes between the glob
# and its awk aborts the guard outright, and one that appears or disappears
# moves `checked` away from `expected`. Both were observed here, against
# the live working tree, while a concurrent writer touched it: renaming a
# package directory out from under the scan failed 7 runs in 12, and
# creating and removing one mid-scan failed 1 in 12. Neither was a
# membership gap; both were the guard reading a tree that was still moving.
#
# So read the tree ONCE and answer every question from that copy. Reading
# one side live and the other from the copy would only move the race, so
# both sides are copied in the same breath. It also makes the run
# self-consistent when the caller points the guard at a tree under active
# development, which is the normal case for a ROOT override.

# Checked before the snapshot so a bad path is reported in the caller's own
# terms, rather than as a copy failure against a temporary file.
[ -f "$CHECKS_FILE" ] || fail "bazel/checks/BUILD.bazel not found (is ROOT set?)"

SNAPSHOT=$(mktemp -d "${TMPDIR:-/tmp}/rust-main-suite-guard.XXXXXXXX")
trap 'rm -rf "$SNAPSHOT"' EXIT
# The copy is all-or-nothing: a plain `cp -R` over a tree that is moving
# leaves a half-populated snapshot behind and exits non-zero, and a
# half-populated snapshot answers every count question wrongly. So each
# attempt copies into a staging directory and is adopted only if the whole
# copy succeeded; otherwise the attempt is discarded and retried. A writer
# that renames a package directory out from under the copy fails that copy,
# so the retry is what turns a moving tree into a completed snapshot rather
# than a wrong answer. The attempt cap is bounded so a writer that never
# stops still ends the run instead of spinning.
SNAPSHOT_ATTEMPTS=${SNAPSHOT_ATTEMPTS:-5}
snapshot_taken=false
attempt=1
while [ "$attempt" -le "$SNAPSHOT_ATTEMPTS" ]; do
    staging="$SNAPSHOT/staging.$attempt"
    rm -rf "$staging"
    if cp "$CHECKS_FILE" "$staging-checks.bazel" \
        && mkdir -p "$staging" \
        && cp -R "$PKGS_ROOT/." "$staging"; then
        snapshot_taken=true
        break
    fi
    rm -rf "$staging" "$staging-checks.bazel"
    attempt=$((attempt + 1))
done
[ "$snapshot_taken" = true ] || fail "could not read a stable snapshot of $PKGS_ROOT in $SNAPSHOT_ATTEMPTS attempts (is the tree being written?)"
mv "$staging" "$SNAPSHOT/packages"
mv "$staging-checks.bazel" "$SNAPSHOT/checks.bazel"
CHECKS_FILE="$SNAPSHOT/checks.bazel"
PKGS_ROOT="$SNAPSHOT/packages"

# Retired owners that keep an all-tests aggregate but are intentionally absent
# from rust-main-packages (AGENTS.md retirement rule). Empty today: the last
# retired owner (d2b-realm-core) was deleted outright.
EXCLUDED_PKGS=${EXCLUDED_PKGS-""}

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
    excluded_block=$(awk '
        {
            line = $0
            sub(/#.*/, "", line)
            opens = gsub(/\(/, "(", line) + 0
            closes = gsub(/\)/, ")", line) + 0
        }
        !in_block && /test_suite\(/ {
            buf = $0
            sub(/#.*/, "", buf)
            depth = opens - closes
            in_block = 1
            if (depth <= 0) {
                if (buf ~ /name = "all-tests"/) print buf
                in_block = 0
            }
            next
        }
        in_block {
            buf = buf "\n" $0
            depth += opens - closes
            if (depth <= 0) {
                if (buf ~ /name = "all-tests"/) print buf
                in_block = 0
            }
        }
        END {
            if (in_block && buf ~ /name = "all-tests"/) print buf
        }
    ' "$excluded_build")
    if [ -z "$excluded_block" ]; then
        fail "excluded package $excluded: all-tests aggregate missing (remove it from EXCLUDED_PKGS?)"
    fi
done

# Core assertion, factored out so negative probes exercise the same logic.
checked=0
check_pkg() {
    local build=$1
    local pkg=$2

    [ -f "$build" ] || return 0
    # Capture the all-tests rule block (paren-depth based, so a file-final
    # block, a one-line aggregate, or an indented closing paren all still
    # emit the block). The aggregate is matched as a rule block, not as a
    # whole-file substring, so a `name = "all-tests"` string elsewhere in
    # the file cannot stand in for the rule.
    block=$(awk '
        {
            line = $0
            sub(/#.*/, "", line)
            opens = gsub(/\(/, "(", line) + 0
            closes = gsub(/\)/, ")", line) + 0
        }
        !in_block && /test_suite\(/ {
            buf = $0
            sub(/#.*/, "", buf)
            depth = opens - closes
            in_block = 1
            if (depth <= 0) {
                if (buf ~ /name = "all-tests"/) print buf
                in_block = 0
            }
            next
        }
        in_block {
            buf = buf "\n" $0
            depth += opens - closes
            if (depth <= 0) {
                if (buf ~ /name = "all-tests"/) print buf
                in_block = 0
            }
        }
        END {
            if (in_block && buf ~ /name = "all-tests"/) print buf
        }
    ' "$build")
    if [ -z "$block" ]; then
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
    # and `tags` cannot delete the rest of the block. The block capture is
    # paren-depth based, so a file-final block, a one-line aggregate, or an
    # indented closing paren all still emit the block (a column-0 close is
    # not assumed).
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
