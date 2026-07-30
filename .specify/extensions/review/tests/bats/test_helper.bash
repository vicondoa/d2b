#!/usr/bin/env bash
# Shared test helper for BATS tests

# Load bats libraries.
#
# Prefer a vendored copy under tests/bats/lib when one exists, so a checkout
# that carries the libraries keeps working unchanged. Otherwise fall back to
# `bats_load_library`, which resolves through BATS_LIB_PATH and lets the
# libraries come from the environment - on this project that is Nix, via
#   nix build nixpkgs#bats.libraries.bats-support nixpkgs#bats.libraries.bats-assert
# Without this fallback the whole suite fails at load time, before any test
# body runs, on a checkout that does not vendor them.
_bats_lib_dir="$(dirname "$BATS_TEST_FILENAME")/lib"
if [[ -d "${_bats_lib_dir}/bats-support" && -d "${_bats_lib_dir}/bats-assert" ]]; then
    _bats_lib_dir="$(cd "${_bats_lib_dir}" && pwd)"
    load "${_bats_lib_dir}/bats-support/load"
    load "${_bats_lib_dir}/bats-assert/load"
else
    bats_load_library bats-support
    bats_load_library bats-assert
fi

# Project root (repo root)
PROJECT_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"

# Scripts under test
SCRIPTS_DIR="${PROJECT_ROOT}/scripts/bash"

# Create a temporary working directory for each test
setup_temp_dir() {
    TEST_TEMP_DIR="$(mktemp -d)"
    export TEST_TEMP_DIR
}

# Clean up the temporary directory
teardown_temp_dir() {
    if [[ -n "${TEST_TEMP_DIR:-}" && -d "$TEST_TEMP_DIR" ]]; then
        rm -rf "$TEST_TEMP_DIR"
    fi
}

# Initialize a git repo in the temp directory
init_git_repo() {
    local dir="${1:-$TEST_TEMP_DIR}"
    git -C "$dir" init --quiet -b main
    git -C "$dir" config user.email "test@example.com"
    git -C "$dir" config user.name "Test"
    # Create initial commit so git diff works
    touch "$dir/.gitkeep"
    git -C "$dir" add .
    git -C "$dir" commit --quiet -m "Initial commit"
}

# Initialize a git repo with a bare remote (for origin/* refs)
init_git_repo_with_remote() {
    local dir="${1:-$TEST_TEMP_DIR}"
    local bare_dir="${dir}/_bare_remote"

    # Create a bare repo to act as origin (explicitly use main)
    mkdir -p "$bare_dir"
    git -C "$bare_dir" init --bare --quiet
    git -C "$bare_dir" symbolic-ref HEAD refs/heads/main

    # Create the working repo
    git -C "$dir" init --quiet -b main
    git -C "$dir" config user.email "test@example.com"
    git -C "$dir" config user.name "Test"
    git -C "$dir" remote add origin "$bare_dir"

    # Create initial commit and push to establish origin/main
    touch "$dir/.gitkeep"
    git -C "$dir" add .
    git -C "$dir" commit --quiet -m "Initial commit"
    git -C "$dir" push --quiet origin main

    # Set origin/HEAD so symbolic-ref works
    git -C "$dir" remote set-head origin --auto 2>/dev/null || true
}

# Validate that output is valid JSON
#
# These three helpers use jq rather than python3. The suite previously shelled
# out to `python3 -m json.tool`, which is absent on a NixOS host that has not
# put a Python into scope; every JSON assertion then failed with "Invalid JSON"
# against output that was in fact valid, blaming the script under test for a
# missing interpreter. jq is already a hard dependency of the surrounding
# tooling, so requiring it costs nothing new. Each helper fails closed with a
# clear message when jq itself is missing, rather than reporting a parse error.
_require_jq() {
    command -v jq > /dev/null 2>&1 \
        || fail "jq is required by these tests but is not on PATH"
}

assert_valid_json() {
    local output="$1"
    _require_jq
    # Plain `jq .`, not `jq -e .`. The -e flag sets a nonzero exit for a
    # parsed value that is null or false, which would report syntactically
    # valid JSON as invalid. This assertion is purely syntactic, matching the
    # `python3 -m json.tool` it replaced.
    echo "$output" | jq . > /dev/null 2>&1 \
        || fail "Invalid JSON: $output"
}

# Extract a JSON field value (simple top-level string/bool/number)
json_field() {
    local json="$1"
    local field="$2"
    _require_jq
    # `has($f)` rather than `.[$f] // ""`. The alternative operator treats
    # false the same as absent, so a field whose value is legitimately false
    # would be indistinguishable from a missing one - the same conflation the
    # python `.get(field, '')` this replaces did not make.
    echo "$json" | jq -r --arg f "$field" 'if has($f) then .[$f] else "" end'
}

# Extract a JSON array field as newline-separated values
json_array_field() {
    local json="$1"
    local field="$2"
    _require_jq
    # `has($f)` again, and deliberately no `// []` coalesce: a field that is
    # present but not iterable must raise rather than silently yield zero
    # items, matching the TypeError the python loop this replaces would have
    # raised. Masking a type mismatch here would let an assertion pass
    # vacuously, which is worse than the helper failing outright.
    echo "$json" | jq -r --arg f "$field" 'if has($f) then .[$f] else [] end | .[]'
}
