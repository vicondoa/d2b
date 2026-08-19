load("@bazel_skylib//rules:native_binary.bzl", "native_test")

_NIX_TAGS = [
    "exclusive",
    "no-remote-cache",
    "no-remote-exec",
    "no-sandbox",
]

_NIX_TOOL_LABELS = [
    "@nix//:bin/nix",
    "@nix_unit//:bin/nix-unit",
    "@python3//:bin/python3",
    "//:flake.nix",
    "//:tests/tools/peak-rss.py",
]

_NIX_TEST_SCRIPT = """#!/bin/sh
set -eu

runfile() {
    key=$1
    if [ -n "${RUNFILES_DIR:-}" ] && [ -e "$RUNFILES_DIR/$key" ]; then
        printf '%s\\n' "$RUNFILES_DIR/$key"
        return 0
    fi
    if [ -n "${RUNFILES_MANIFEST_FILE:-}" ] && [ -r "$RUNFILES_MANIFEST_FILE" ]; then
        while IFS= read -r line || [ -n "$line" ]; do
            case "$line" in
                "$key "*)
                    printf '%s\\n' "${line#"$key "}"
                    return 0
                    ;;
            esac
        done <"$RUNFILES_MANIFEST_FILE"
    fi
    if [ -e "$key" ]; then
        printf '%s\\n' "$key"
        return 0
    fi
    printf 'Bazel runfile is missing: %s\\n' "$key" >&2
    return 1
}

NIX_BIN=$(runfile "__NIX_KEY__")
PYTHON_BIN=$(runfile "__PYTHON_KEY__")
PEAK_RSS=$(runfile "__PEAK_KEY__")
FLAKE_PATH=$(runfile "__FLAKE_KEY__")
ROOT=${FLAKE_PATH%/flake.nix}
if [ "$ROOT" = "$FLAKE_PATH" ]; then
    ROOT=.
fi
ROOT=$(CDPATH= cd -- "$ROOT" && pwd -P)
if [ -n "${D2B_REPO_ROOT:-}" ]; then
    ROOT=$D2B_REPO_ROOT
fi
cd "$ROOT"
export ROOT
export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"
if [ -n "${D2B_REPO_ROOT:-}" ]; then
    FLAKE_REF="git+file://$ROOT"
else
    FLAKE_REF="$ROOT"
fi
SYSTEM=$("$NIX_BIN" eval --raw --impure --expr builtins.currentSystem)
__SMOKE_ASSIGNMENT__
__NIX_UNIT_CHECK__
__EVAL_JOBS_ASSIGNMENT__
exec "$PYTHON_BIN" "$PEAK_RSS" \
    --lane "__LANE__" \
    --max-kib "__MAX_KIB__" \
    -- __TOOL__ __COMMAND__
"""

def nix_native_test(name, command, lane, max_kib, data, uses_nix_unit = False, uses_eval_jobs = False, smoke = False, timeout = "long"):
    runner = name + "_runner.sh"
    script = _NIX_TEST_SCRIPT
    script = script.replace("__NIX_KEY__", "$(rlocationpath @nix//:bin/nix)")
    script = script.replace("__PYTHON_KEY__", "$(rlocationpath @python3//:bin/python3)")
    script = script.replace("__PEAK_KEY__", "$(rlocationpath //:tests/tools/peak-rss.py)")
    script = script.replace("__FLAKE_KEY__", "$(rlocationpath //:flake.nix)")
    script = script.replace("__LANE__", lane)
    script = script.replace("__MAX_KIB__", str(max_kib))
    script = script.replace("__COMMAND__", command)
    if smoke:
        script = script.replace(
            "__SMOKE_ASSIGNMENT__",
            'SMOKE_PATH=$(runfile "$(rlocationpath //:tests/unit/smoke/smoke-eval-aarch64.nix)")',
        )
    else:
        script = script.replace("__SMOKE_ASSIGNMENT__", "")
    if uses_nix_unit:
        script = script.replace(
            "__NIX_UNIT_CHECK__",
            'NIX_UNIT_BIN=$(runfile "$(rlocationpath @nix_unit//:bin/nix-unit)")\n"$NIX_UNIT_BIN" --help >/dev/null',
        )
    else:
        script = script.replace("__NIX_UNIT_CHECK__", "")
    if uses_eval_jobs:
        script = script.replace(
            "__EVAL_JOBS_ASSIGNMENT__",
            'EVAL_JOBS_BIN=$(runfile "$(rlocationpath @nix_eval_jobs//:bin/nix-eval-jobs)")',
        )
        script = script.replace("__TOOL__", '"$EVAL_JOBS_BIN"')
    else:
        script = script.replace("__EVAL_JOBS_ASSIGNMENT__", "")
        script = script.replace("__TOOL__", '"$NIX_BIN"')
    make_vars = {
        "$(rlocationpath @nix//:bin/nix)": "__NIX_RUNFILE__",
        "$(rlocationpath @nix_unit//:bin/nix-unit)": "__NIX_UNIT_RUNFILE__",
        "$(rlocationpath @nix_eval_jobs//:bin/nix-eval-jobs)": "__EVAL_JOBS_RUNFILE__",
        "$(rlocationpath @python3//:bin/python3)": "__PYTHON_RUNFILE__",
        "$(rlocationpath //:flake.nix)": "__FLAKE_RUNFILE__",
        "$(rlocationpath //:tests/tools/peak-rss.py)": "__PEAK_RUNFILE__",
        "$(rlocationpath //:tests/unit/smoke/smoke-eval-aarch64.nix)": "__SMOKE_RUNFILE__",
    }
    for make_var, placeholder in make_vars.items():
        script = script.replace(make_var, placeholder)
    script = script.replace("$", "$$")
    for make_var, placeholder in make_vars.items():
        script = script.replace(placeholder, make_var)
    native.genrule(
        name = name + "_runner",
        outs = [runner],
        tools = _NIX_TOOL_LABELS + ([
            "@nix_eval_jobs//:bin/nix-eval-jobs",
        ] if uses_eval_jobs else []) + ([
            "//:tests/unit/smoke/smoke-eval-aarch64.nix",
        ] if smoke else []),
        cmd = "cat > \"$(OUTS)\" <<'EOF'\n%s\nEOF\nchmod +x \"$(OUTS)\"" % script,
    )
    native_test(
        name = name,
        src = ":" + runner,
        data = data + [
            "//:flake.nix",
            "//:tests/tools/peak-rss.py",
            "@nix//:bin/nix",
            "@python3//:bin/python3",
        ] + ([
            "@nix_unit//:bin/nix-unit",
        ] if uses_nix_unit else []) + ([
            "@nix_eval_jobs//:bin/nix-eval-jobs",
        ] if uses_eval_jobs else []) + ([
            "//:tests/unit/smoke/smoke-eval-aarch64.nix",
        ] if smoke else []),
        tags = _NIX_TAGS,
        timeout = timeout,
    )

def nix_unit_case_test(case_name, data):
    nix_native_test(
        name = "nix-unit-case-" + case_name,
        command = 'eval --no-write-lock-file --raw "$FLAKE_REF#nixUnitJobs.${SYSTEM}.case-' + case_name + '.drvPath" >/dev/null',
        lane = "nix-unit-case-" + case_name,
        max_kib = 10683720,
        data = data,
        uses_nix_unit = True,
    )

def nix_unit_case_tests(groups, data):
    for group_cases in groups.values():
        for case_name in group_cases:
            nix_unit_case_test(case_name, data)
