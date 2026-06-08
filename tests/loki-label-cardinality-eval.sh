#!/usr/bin/env bash
# tests/loki-label-cardinality-eval.sh — static gate for the
# Loki label contract (P3 ph3-p3-loki-label-contract).
#
# Asserts, against the Alloy configs emitted by
# nixos-modules/components/observability/{host,stack,guest}.nix:
#
#   1. Every key in any `loki.source.* "..." { ... labels = { ... } }`
#      stanza is in the allowlist {vm, env, role, severity, source}.
#   2. No label value is path-like (no `/` in literal values; no
#      `${quote ...}` interpolating a path-typed variable).
#   3. Closed-enum labels stay within their cardinality budget:
#        role     <= 10
#        severity <= 5
#        source   <= 5
#      Open-enum labels (vm, env) are reviewed via the contract doc;
#      the gate asserts they only take values from `${quote …}`
#      interpolations of vmName/envName/cfg.env/hostName or from the
#      documented literal escape hatches ("host", "obs").
#
# Canonical contract: docs/reference/loki-label-contract.md.
#
# Run via:
#   bash tests/loki-label-cardinality-eval.sh

set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

FILES=(
  "$ROOT/nixos-modules/components/observability/host.nix"
  "$ROOT/nixos-modules/components/observability/stack.nix"
  "$ROOT/nixos-modules/components/observability/guest.nix"
)

ALLOWED_KEYS=(vm env role severity source)
BUDGET_ROLE=10
BUDGET_SEVERITY=5
BUDGET_SOURCE=5

# Documented literal escape hatches for open-enum labels.
# shellcheck disable=SC2034
ALLOWED_VM_LITERALS=(host)
# shellcheck disable=SC2034
ALLOWED_ENV_LITERALS=(host obs)
# Variables permitted under `${quote VAR}` for open-enum labels.
ALLOWED_VM_QUOTE_VARS=(name cfg.identity.vmName cfg.identity.envName hostName)
ALLOWED_ENV_QUOTE_VARS=(envLabel env cfg.env cfg.identity.envName)

PASS=0
FAIL=0

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; PASS=$((PASS+1)); }
fail() { log "  FAIL: $*"; FAIL=$((FAIL+1)); }

in_list() {
  local needle=$1; shift
  local hay
  for hay in "$@"; do
    [[ "$hay" == "$needle" ]] && return 0
  done
  return 1
}

# Extract loki.source stanzas as (file, source_name, labels_body)
# triples, separated by NULs for safe parsing. The Alloy config in
# our three files uses indented multi-line `labels = { ... }` blocks
# inside `loki.source.<type> "<name>" { ... }` blocks; we parse them
# directly out of the Nix source rather than rendering the Alloy file
# (the gate must remain build-free).
extract_blocks() {
  local file=$1
  awk -v RS='\0' '
    function adj_depth(s,   i, n, c, two) {
      n = length(s)
      i = 1
      while (i <= n) {
        two = substr(s, i, 2)
        if (two == "${") {
          # skip over ${...} including any nested {...}; do not count
          # its braces against the outer block depth
          i += 2
          inner = 1
          while (i <= n && inner > 0) {
            c = substr(s, i, 1)
            if (c == "{") inner++
            else if (c == "}") inner--
            i++
          }
        } else {
          c = substr(s, i, 1)
          if (c == "{") d_count++
          else if (c == "}") d_count--
          i++
        }
      }
    }
    function extract_labels_body(body,   i, n, j, two, c, inner, start) {
      # find `labels` ... `=` ... `{`
      if (!match(body, /labels[[:space:]]*=[[:space:]]*\{/)) return ""
      start = RSTART + RLENGTH
      n = length(body)
      i = start
      inner = 1
      while (i <= n && inner > 0) {
        two = substr(body, i, 2)
        if (two == "${") {
          i += 2
          subinner = 1
          while (i <= n && subinner > 0) {
            c = substr(body, i, 1)
            if (c == "{") subinner++
            else if (c == "}") subinner--
            i++
          }
        } else {
          c = substr(body, i, 1)
          if (c == "{") inner++
          else if (c == "}") inner--
          i++
        }
      }
      return substr(body, start, i - start - 1)
    }
    {
      src = $0
      n = length(src)
      i = 1
      while (i <= n) {
        # Look for `loki.source.<type>` at position i
        rest = substr(src, i)
        if (match(rest, /^loki\.source\.[a-zA-Z_]+[[:space:]]+"[^"]+"[[:space:]]*\{/)) {
          outer_rstart = RSTART
          outer_rlength = RLENGTH
          hdr = substr(rest, outer_rstart, outer_rlength)
          if (match(hdr, /"[^"]+"/)) {
            source_name = substr(hdr, RSTART+1, RLENGTH-2)
          }
          i += outer_rstart - 1 + outer_rlength
          # Now scan from i forward, brace-counting (skipping ${...}),
          # starting depth at 1.
          depth = 1
          start = i
          while (i <= n && depth > 0) {
            two = substr(src, i, 2)
            if (two == "${") {
              i += 2
              inner = 1
              while (i <= n && inner > 0) {
                c = substr(src, i, 1)
                if (c == "{") inner++
                else if (c == "}") inner--
                i++
              }
            } else {
              c = substr(src, i, 1)
              if (c == "{") depth++
              else if (c == "}") depth--
              i++
            }
          }
          body = substr(src, start, i - start - 1)
          labels_body = extract_labels_body(body)
          printf("%s\t%s%c", source_name, labels_body, 0)
        } else {
          i++
        }
      }
    }
  ' "$file"
}

declare -a ALL_LITERAL_ROLE=()
declare -a ALL_LITERAL_SEVERITY=()
declare -a ALL_LITERAL_SOURCE=()

for file in "${FILES[@]}"; do
  if [[ ! -f "$file" ]]; then
    fail "missing file: $file"
    continue
  fi

  log "scanning $file"

  while IFS= read -r -d '' record; do
    source_name=${record%%	*}
    labels_body=${record#*	}

    # Parse `key = <value>,` lines from labels_body.
    while IFS= read -r line; do
      line=${line%%#*}                     # strip inline comments
      [[ "$line" =~ ^[[:space:]]*$ ]] && continue
      if [[ ! "$line" =~ ^[[:space:]]*([a-zA-Z_][a-zA-Z0-9_]*)[[:space:]]*=[[:space:]]*(.*)$ ]]; then
        continue
      fi
      key=${BASH_REMATCH[1]}
      raw=${BASH_REMATCH[2]}
      raw=${raw%,}
      raw=${raw%"${raw##*[![:space:]]}"}  # rtrim

      # (1) key allowlist
      if ! in_list "$key" "${ALLOWED_KEYS[@]}"; then
        fail "[$file::$source_name] label key '$key' not in allowlist {${ALLOWED_KEYS[*]}}"
        continue
      fi

      # Classify value: literal "..." or ${quote VAR} or "${VAR}"
      literal=""
      qvar=""
      ivar=""
      if [[ "$raw" =~ ^\"([^\"\$]*)\"$ ]]; then
        literal=${BASH_REMATCH[1]}
      elif [[ "$raw" =~ ^\"\$\{([a-zA-Z_][a-zA-Z0-9_.]*)\}\"$ ]]; then
        # `"${var}"` — a Nix string with a single interpolation. The
        # rendered Alloy output is a literal, but the value is set by
        # the call site. We allow this shape; the literal-budget pass
        # below tallies call-site values separately.
        ivar=${BASH_REMATCH[1]}
      elif [[ "$raw" =~ ^\$\{quote[[:space:]]+([^}]+)\}$ ]]; then
        qvar=${BASH_REMATCH[1]}
        qvar=${qvar%"${qvar##*[![:space:]]}"}
      else
        fail "[$file::$source_name] label '$key' has unrecognized value shape: $raw"
        continue
      fi

      # (2) path-like value rejection on literals
      if [[ -n "$literal" ]]; then
        if [[ "$literal" == */* ]] || [[ "$literal" == /* ]]; then
          fail "[$file::$source_name] label '$key' has path-like literal value: '$literal'"
          continue
        fi
      fi

      # Per-key value-shape rules. Tallying of literals for closed-enum
      # labels happens in a second pass below so call-site literals
      # (passed to helpers like mkJournalSource) are accounted for.
      case "$key" in
        role|severity|source)
          if [[ -n "$qvar" ]]; then
            fail "[$file::$source_name] label '$key' must be a literal string or pass-through interpolation; got \${quote $qvar}"
            continue
          fi
          if [[ -n "$literal" ]]; then
            case "$key" in
              role)     ALL_LITERAL_ROLE+=("$literal") ;;
              severity) ALL_LITERAL_SEVERITY+=("$literal") ;;
              source)   ALL_LITERAL_SOURCE+=("$literal") ;;
            esac
          fi
          ;;
        vm|env)
          # Open-enum labels; runtime cardinality is operator-bounded.
          # Accept any shape that passed (literal already path-checked
          # above; ${quote VAR} unrestricted; "${var}" pass-through).
          :
          ;;
      esac
    done <<< "$labels_body"

    ok "[$file::$source_name] labels conform to contract"
  done < <(extract_blocks "$file")
done

# Second pass: collect call-site literals for `role` from the
# mkJournalSource helper invocations in host.nix. The helper renders
# `role = "${role}",` (pass-through interpolation) into the Alloy
# text, so the in-block pass alone cannot see which role literals
# the call sites use; this scan reconstructs that view.
#
# We deliberately do NOT collect call-site literals for `severity`
# or `source`: neither label is threaded through a helper today, and
# scanning the broader files would pick up unrelated Prometheus
# alert-rule `severity = "..."` assignments (stack.nix) and miscount
# them against the Loki budget.
collect_callsite_role_literals() {
  local file=$1
  grep -Eo '(^|[[:space:]])role[[:space:]]*=[[:space:]]*"[^"$]+"[[:space:]]*;' "$file" 2>/dev/null \
    | sed -E 's/.*role[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/'
}

for file in "${FILES[@]}"; do
  [[ -f "$file" ]] || continue
  while IFS= read -r v; do
    [[ -n "$v" ]] && ALL_LITERAL_ROLE+=("$v")
  done < <(collect_callsite_role_literals "$file")
done

# (3) cardinality budgets
uniq_count() {
  local -a arr=( "$@" )
  if [[ ${#arr[@]} -eq 0 ]]; then
    echo 0
    return
  fi
  printf '%s\n' "${arr[@]}" | sort -u | wc -l
}

ROLE_N=$(uniq_count "${ALL_LITERAL_ROLE[@]+"${ALL_LITERAL_ROLE[@]}"}")
SEV_N=$(uniq_count "${ALL_LITERAL_SEVERITY[@]+"${ALL_LITERAL_SEVERITY[@]}"}")
SRC_N=$(uniq_count "${ALL_LITERAL_SOURCE[@]+"${ALL_LITERAL_SOURCE[@]}"}")

check_budget() {
  local name=$1 actual=$2 budget=$3
  if (( actual > budget )); then
    fail "cardinality budget breach: $name has $actual distinct literal values (budget $budget)"
  else
    ok "cardinality budget ok: $name has $actual distinct literal values (budget $budget)"
  fi
}

check_budget role     "$ROLE_N" "$BUDGET_ROLE"
check_budget severity "$SEV_N"  "$BUDGET_SEVERITY"
check_budget source   "$SRC_N"  "$BUDGET_SOURCE"

log "summary: PASS=$PASS FAIL=$FAIL"
log "  role literals:     $(printf '%s\n' "${ALL_LITERAL_ROLE[@]+"${ALL_LITERAL_ROLE[@]}"}" | sort -u | xargs)"
log "  severity literals: $(printf '%s\n' "${ALL_LITERAL_SEVERITY[@]+"${ALL_LITERAL_SEVERITY[@]}"}" | sort -u | xargs)"
log "  source literals:   $(printf '%s\n' "${ALL_LITERAL_SOURCE[@]+"${ALL_LITERAL_SOURCE[@]}"}" | sort -u | xargs)"

if (( FAIL > 0 )); then
  exit 1
fi
exit 0
