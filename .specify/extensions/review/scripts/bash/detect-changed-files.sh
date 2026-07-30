#!/usr/bin/env bash
# Detect changed files for code review via git diff
#
# Identifies changed files by comparing the current branch against
# a base ref plus any uncommitted work (Mode A - feature branch
# diff + working directory) or by collecting staged + unstaged changes
# (Mode B - working directory changes only).
#
# The base ref is resolved in this order:
#   1. --base <ref>                (explicit CLI override; wins)
#   2. $SPECIFY_REVIEW_BASE_REF    (explicit environment override)
#   3. git symbolic-ref refs/remotes/origin/HEAD
#   4. origin/main
#   5. origin/master
#
# An explicit base ref (1 or 2) is validated with `git rev-parse --verify`.
# If it cannot be resolved the script fails with exit 1 rather than silently
# falling back to the repository default branch: reviews of an integration
# lineage that never merges to the default branch (for example the ADR-046
# `v3` lineage) would otherwise be scoped against unrelated history.
#
# Usage: ./detect-changed-files.sh [OPTIONS]
#
# OPTIONS:
#   --base <ref>  Explicit diff base ref (branch, tag or commit)
#   --json        Output in JSON format (for machine consumption)
#   --help, -h    Show this help message
#
# ENVIRONMENT:
#   SPECIFY_REVIEW_BASE_REF   Explicit diff base ref; overridden by --base
#
# EXIT CODES:
#   0  Changed files detected successfully
#   1  Error (git unavailable, not a git repository, unresolvable base ref)
#   2  No changes detected
#
# OUTPUTS:
#   Text mode:
#     BRANCH: <current-branch>
#     DEFAULT_BRANCH: <ref actually used as the diff base>
#     BASE_SOURCE: <how the base ref was resolved>
#     MODE: <detection mode description>
#     CHANGED_FILES:
#       file1
#       file2
#
#   JSON mode:
#     {"branch":"...","default_branch":"...","base_source":"...","mode":"...","changed_files":["..."]}

set -e

# --- Helper: escape a string for safe JSON embedding ---
json_escape() {
    local s="$1"
    s="${s//\\/\\\\}"   # backslash
    s="${s//\"/\\\"}"   # double quote
    s="${s//$'\t'/\\t}"    # tab
    s="${s//$'\n'/\\n}"    # newline
    s="${s//$'\r'/\\r}"    # carriage return
    printf '%s' "$s"
}

# --- Helper: output error and exit ---
# Defined before argument parsing so that CLI parse errors honour --json too.
error_exit() {
    local message="$1"
    local code="${2:-1}"
    if $JSON_MODE; then
        printf '{"error":"%s"}\n' "$(json_escape "$message")"
    else
        echo "Error: $message" >&2
    fi
    exit "$code"
}

# --- Argument parsing ---
BASE_REF_ARG=""
BASE_SOURCE=""

# Pre-scan for --json so that JSON_MODE is known for every subsequent error,
# regardless of where --json appears in the argument vector (for example
# '--base --json' or '--bogus --json').
JSON_MODE=false
for _arg in "$@"; do
    if [[ "$_arg" == "--json" ]]; then
        JSON_MODE=true
        break
    fi
done

while [[ $# -gt 0 ]]; do
    case "$1" in
        --json) JSON_MODE=true ;;
        --base)
            # A following option-looking token means the value was omitted, so
            # `--base --json` reports a missing value rather than resolving a
            # ref literally named `--json`. A bare `-` is exempt: git accepts
            # it as shorthand for the previous branch (`@{-1}`), so it is a
            # legitimate base ref rather than a stray flag.
            if [[ $# -lt 2 || -z "$2" || ( "$2" == -* && "$2" != "-" ) ]]; then
                error_exit "--base requires a ref argument. Re-run as '--base <ref>' naming an existing branch, tag or commit: for ADR-046 waves that is the integration lineage 'v3', or the predecessor wave branch when the wave is stacked. List candidates with 'git branch -a' and confirm one with 'git rev-parse --verify <ref>'." 1
            fi
            BASE_REF_ARG="$2"
            BASE_SOURCE="cli"
            shift
            ;;
        --base=*)
            BASE_REF_ARG="${1#--base=}"
            if [[ -z "$BASE_REF_ARG" ]]; then
                error_exit "--base requires a ref argument. Re-run as '--base <ref>' naming an existing branch, tag or commit: for ADR-046 waves that is the integration lineage 'v3', or the predecessor wave branch when the wave is stacked. List candidates with 'git branch -a' and confirm one with 'git rev-parse --verify <ref>'." 1
            fi
            BASE_SOURCE="cli"
            ;;
        --help|-h)
            cat << 'EOF'
Usage: detect-changed-files.sh [OPTIONS]

Detect changed files for code review via git diff.

OPTIONS:
  --base <ref>  Explicit diff base ref (branch, tag or commit). Overrides
                SPECIFY_REVIEW_BASE_REF and the repository default branch.
  --json        Output in JSON format
  --help, -h    Show this help message

ENVIRONMENT:
  SPECIFY_REVIEW_BASE_REF   Explicit diff base ref; --base takes precedence.

BASE REF RESOLUTION ORDER:
  --base > SPECIFY_REVIEW_BASE_REF > origin/HEAD > origin/main > origin/master

An explicit base ref that cannot be resolved is a hard error (exit 1); the
script never silently falls back to the repository default branch. Reviews of
an integration lineage that does not merge to the default branch (for example
the ADR-046 'v3' lineage) MUST pass --base naming that lineage, or naming the
predecessor wave branch when the wave is stacked.

EXIT CODES:
  0  Changed files detected successfully
  1  Error (git unavailable, not a git repository, unresolvable base ref)
  2  No changes detected
EOF
            exit 0
            ;;
        *) error_exit "Unknown option '$1'. Run 'detect-changed-files.sh --help' to list the supported options (--base <ref>, --json, --help)." 1 ;;
    esac
    shift
done

if [[ -z "$BASE_REF_ARG" && -n "${SPECIFY_REVIEW_BASE_REF:-}" ]]; then
    BASE_REF_ARG="$SPECIFY_REVIEW_BASE_REF"
    BASE_SOURCE="env"
fi

# --- Helper: format bash array as JSON array ---
fmt_array() {
    local arr=("$@")
    if [[ ${#arr[@]} -eq 0 ]]; then echo "[]"; return; fi
    local first=true
    local result="["
    for item in "${arr[@]}"; do
        if $first; then first=false; else result+=","; fi
        result+="\"$(json_escape "$item")\""
    done
    result+="]"
    echo "$result"
}

# --- 1a. Verify Git Availability ---
if ! command -v git >/dev/null 2>&1; then
    error_exit "git is not available. The review extension requires git to identify changed files. Install git and confirm it is on PATH with 'command -v git', or tell the review agent explicitly which files to review instead of relying on detection." 1
fi

if ! git rev-parse --git-dir >/dev/null 2>&1; then
    error_exit "Not a git repository. The review extension requires git to identify changed files. Re-run this script from inside the repository worktree you want reviewed (check with 'git rev-parse --show-toplevel'), or tell the review agent explicitly which files to review." 1
fi

# --- 1b. Detect Branch Context ---

# Get current branch (empty string if detached HEAD)
CURRENT_BRANCH=$(git branch --show-current 2>/dev/null || echo "")

# Determine the diff base ref.
# DEFAULT_BRANCH holds the ref ACTUALLY used as the diff base (field name kept
# for output compatibility). BASE_REV is the resolved rev handed to git.
DEFAULT_BRANCH=""
BASE_REV=""

if [[ -n "$BASE_REF_ARG" ]]; then
    # Explicit override: validate, never fall back.
    if git rev-parse --verify --quiet "$BASE_REF_ARG" >/dev/null 2>&1; then
        DEFAULT_BRANCH="$BASE_REF_ARG"
        BASE_REV="$BASE_REF_ARG"
    elif git rev-parse --verify --quiet "origin/$BASE_REF_ARG" >/dev/null 2>&1; then
        DEFAULT_BRANCH="$BASE_REF_ARG"
        BASE_REV="origin/$BASE_REF_ARG"
    else
        error_exit "Cannot resolve base ref '${BASE_REF_ARG}' (source: ${BASE_SOURCE}). Tried '${BASE_REF_ARG}' and 'origin/${BASE_REF_ARG}'. Refusing to fall back to the repository default branch. Re-run with --base naming an existing branch, tag or commit: for ADR-046 waves that is the integration lineage 'v3', or the predecessor wave branch when the wave is stacked. List candidates with 'git branch -a' and confirm one with 'git rev-parse --verify <ref>'." 1
    fi
else
    # Try symbolic-ref first
    symref=$(git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null || echo "")
    if [[ -n "$symref" ]]; then
        DEFAULT_BRANCH="${symref##refs/remotes/origin/}"
        BASE_REV="origin/${DEFAULT_BRANCH}"
        BASE_SOURCE="origin-head"
    fi

    # Fallback: check origin/main
    if [[ -z "$DEFAULT_BRANCH" ]]; then
        if git rev-parse --verify --quiet origin/main >/dev/null 2>&1; then
            DEFAULT_BRANCH="main"
            BASE_REV="origin/main"
            BASE_SOURCE="origin-main"
        fi
    fi

    # Fallback: check origin/master
    if [[ -z "$DEFAULT_BRANCH" ]]; then
        if git rev-parse --verify --quiet origin/master >/dev/null 2>&1; then
            DEFAULT_BRANCH="master"
            BASE_REV="origin/master"
            BASE_SOURCE="origin-master"
        fi
    fi

    if [[ -z "$DEFAULT_BRANCH" ]]; then
        BASE_SOURCE="none"
    fi
fi

# --- Helper: collect NUL-delimited git output into COLLECTED ---
#
# A `while read` fed by a process substitution - `done < <(git ...)` - cannot
# fail closed: the process substitution is asynchronous, so bash never
# propagates git's exit status to `set -e` and a failed git (bad ref, corrupt
# index, OOM) is indistinguishable from an empty diff. For a review gate that
# is fail-open: the reviewer is told there is nothing to review when the diff
# could not be computed at all.
#
# Capturing into a shell variable via `$(...)` would check the status but is
# not NUL-safe: bash silently drops NUL bytes from command substitution, which
# destroys the `-z` record separator and mangles filenames containing spaces or
# newlines. So the bytes go to a temporary file instead:
#   - `git ... >"$_out"` is a synchronous simple command, so its exit status is
#     checked explicitly and a failure exits 1;
#   - the file holds the raw NUL-delimited bytes untouched;
#   - `while ... done < "$_out"` is a plain file redirection, not a pipeline or
#     process substitution, so the loop runs in the main shell and the
#     collected array stays visible to the caller.
_DIFF_TMPDIR=""

# shellcheck disable=SC2329  # invoked indirectly by the EXIT trap below
_cleanup_diff_tmpdir() {
    if [[ -n "$_DIFF_TMPDIR" ]]; then
        rm -rf "$_DIFF_TMPDIR"
    fi
}
trap '_cleanup_diff_tmpdir' EXIT

COLLECTED=()

collect_git_nul() {
    COLLECTED=()
    if [[ -z "$_DIFF_TMPDIR" ]]; then
        if ! _DIFF_TMPDIR=$(mktemp -d 2>/dev/null); then
            error_exit "Cannot create a temporary directory to collect git output. Check that TMPDIR is writable and has free space, then re-run." 1
        fi
    fi
    local _out="${_DIFF_TMPDIR}/git-out.bin"
    if ! git "$@" >"$_out" 2>/dev/null; then
        error_exit "git $* failed while collecting changed files. The change set could not be computed, so this run reports an error rather than an empty review scope. Re-run the command by hand to see git's own diagnostics, and check that the base ref still resolves ('git rev-parse --verify <ref>') and that the repository index is intact ('git status')." 1
    fi
    local line
    while IFS= read -r -d '' line; do
        [[ -n "$line" ]] && COLLECTED+=("$line")
    done < "$_out"
}

# --- 1c. Get Changed Files ---

CHANGED_FILES=()
MODE=""

# Mode A applies when a usable base ref resolves to something other than HEAD.
# With an explicit base ref the current branch name is irrelevant: a wave branch
# based on 'v3' selects Mode A, while standing ON 'v3' with '--base v3' resolves
# to HEAD and therefore falls through to Mode B (working directory changes).
USE_MODE_A=false
if [[ -n "$BASE_REV" ]]; then
    if [[ -n "$BASE_REF_ARG" ]]; then
        _base_sha=$(git rev-parse --verify --quiet "$BASE_REV" 2>/dev/null || echo "")
        _head_sha=$(git rev-parse --verify --quiet HEAD 2>/dev/null || echo "")
        if [[ -n "$_base_sha" && -n "$_head_sha" && "$_base_sha" != "$_head_sha" ]]; then
            USE_MODE_A=true
        fi
    elif [[ -n "$CURRENT_BRANCH" && "$CURRENT_BRANCH" != "$DEFAULT_BRANCH" ]]; then
        USE_MODE_A=true
    fi
fi

if $USE_MODE_A; then
    # Mode A - Feature Branch
    MERGE_BASE=$(git merge-base "$BASE_REV" HEAD 2>/dev/null || echo "")

    if [[ -n "$MERGE_BASE" ]]; then
        # Committed changes since merge-base
        collect_git_nul diff --name-only -z --diff-filter=ACMR "${MERGE_BASE}...HEAD"
        COMMITTED=("${COLLECTED[@]}")

        # Staged (index) changes
        collect_git_nul diff --cached --name-only -z --diff-filter=ACMR
        STAGED=("${COLLECTED[@]}")

        # Unstaged (working tree) changes
        collect_git_nul diff --name-only -z --diff-filter=ACMR
        UNSTAGED=("${COLLECTED[@]}")

        # Combine and deduplicate (bash 3 compatible - no associative arrays)
        CHANGED_FILES=()
        for f in "${COMMITTED[@]}" "${STAGED[@]}" "${UNSTAGED[@]}"; do
            [[ -z "$f" ]] && continue
            _dup=false
            for existing in "${CHANGED_FILES[@]}"; do
                if [[ "$existing" == "$f" ]]; then
                    _dup=true
                    break
                fi
            done
            if ! $_dup; then
                CHANGED_FILES+=("$f")
            fi
        done

        MODE="Feature branch diff (${DEFAULT_BRANCH}...HEAD) + uncommitted changes"
    else
        # merge-base failed - fall through to Mode B
        DEFAULT_BRANCH=""
        BASE_SOURCE="none"
    fi
fi

if [[ -z "$MODE" ]]; then
    # Mode B - Working Directory Changes
    collect_git_nul diff --cached --name-only -z --diff-filter=ACMR
    STAGED=("${COLLECTED[@]}")

    collect_git_nul diff --name-only -z --diff-filter=ACMR
    UNSTAGED=("${COLLECTED[@]}")

    # Combine and deduplicate (bash 3 compatible - no associative arrays)
    CHANGED_FILES=()
    for f in "${STAGED[@]}" "${UNSTAGED[@]}"; do
        [[ -z "$f" ]] && continue
        _dup=false
        for existing in "${CHANGED_FILES[@]}"; do
            if [[ "$existing" == "$f" ]]; then
                _dup=true
                break
            fi
        done
        if ! $_dup; then
            CHANGED_FILES+=("$f")
        fi
    done

    MODE="Working directory changes (staged + unstaged)"
    [[ -z "$DEFAULT_BRANCH" ]] && DEFAULT_BRANCH="(unknown)"
    [[ -z "$BASE_SOURCE" ]] && BASE_SOURCE="none"
fi

# --- 1d. Validate Changed Files ---
if [[ ${#CHANGED_FILES[@]} -eq 0 ]]; then
    if $JSON_MODE; then
        printf '{"branch":"%s","default_branch":"%s","base_source":"%s","mode":"%s","changed_files":[],"message":"No changes detected. Nothing to review."}\n' \
            "$(json_escape "$CURRENT_BRANCH")" "$(json_escape "$DEFAULT_BRANCH")" "$(json_escape "$BASE_SOURCE")" "$(json_escape "$MODE")"
    else
        echo "No changes detected. Nothing to review."
    fi
    exit 2
fi

# --- Output ---
if $JSON_MODE; then
    printf '{"branch":"%s","default_branch":"%s","base_source":"%s","mode":"%s","changed_files":%s}\n' \
        "$(json_escape "$CURRENT_BRANCH")" "$(json_escape "$DEFAULT_BRANCH")" "$(json_escape "$BASE_SOURCE")" "$(json_escape "$MODE")" "$(fmt_array "${CHANGED_FILES[@]}")"
else
    echo "BRANCH: $CURRENT_BRANCH"
    echo "DEFAULT_BRANCH: $DEFAULT_BRANCH"
    echo "BASE_SOURCE: $BASE_SOURCE"
    echo "MODE: $MODE"
    echo "CHANGED_FILES:"
    for f in "${CHANGED_FILES[@]}"; do
        echo "  $f"
    done
fi

exit 0
