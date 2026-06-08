#!/usr/bin/env bash
# tests/manpage-completeness-eval.sh— manpage completeness gate.
#
# Asserts that every top-level clap subcommand declared in
# `packages/nixling/src/lib.rs` (`enum NativeCommand { ... }`) is
# documented as a section in the committed nixling(1) manpage at
# `docs/manpages/nixling.1`.
#
# Rationale: clap_mangen emits one `.TP` entry per subcommand under
# the `SUBCOMMANDS` block (rendered as `nixling-<name>(1)`). When a
# new top-level verb lands without rerunning
# `cargo xtask gen-cli-shell-artifacts`, the manpage silently drops
# it. This gate fails closed on that drift independent of the
# byte-diff drift gate (`tests/manpage-completion-drift.sh`), which
# requires a working cargo toolchain to regenerate.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

cli_src="$ROOT/packages/nixling/src/lib.rs"
manpage="$ROOT/docs/manpages/nixling.1"

[ -f "$cli_src" ] || fail "manpage-completeness: missing CLI source $cli_src"
[ -f "$manpage" ] || fail "manpage-completeness: missing manpage $manpage"

# Extract the body of `enum NativeCommand { ... }` from the CLI
# source, then enumerate the kebab-case subcommand names. Two
# forms are recognized:
#   1) An explicit override:   #[command(name = "rotate-known-host")]
#      … on the line immediately preceding the variant.
#   2) The default clap conversion: the variant identifier
#      `RotateKnownHost` becomes `rotate-known-host` (PascalCase →
#      kebab-case lowercased). Variants are detected as
#      `^    Ident(...)` lines inside the enum block.
expected_cmds=$(
  awk '
    /^enum NativeCommand[[:space:]]*\{/ { in_enum = 1; next }
    in_enum && /^\}/ { in_enum = 0; next }
    !in_enum { next }
    # Capture explicit clap rename attributes.
    /^[[:space:]]*#\[command\(name[[:space:]]*=[[:space:]]*"[^"]+"\)\]/ {
      match($0, /"[^"]+"/)
      override = substr($0, RSTART + 1, RLENGTH - 2)
      next
    }
    /^[[:space:]]*[A-Z][A-Za-z0-9_]*\(/ {
      if (override != "") {
        print override
        override = ""
        next
      }
      # Strip leading whitespace + trailing "(...".
      name = $0
      sub(/^[[:space:]]+/, "", name)
      sub(/\(.*/, "", name)
      # PascalCase → kebab-case lowercase.
      out = ""
      for (i = 1; i <= length(name); i++) {
        ch = substr(name, i, 1)
        if (ch ~ /[A-Z]/ && i > 1) {
          out = out "-" tolower(ch)
        } else {
          out = out tolower(ch)
        }
      }
      print out
    }
  ' "$cli_src" | sort -u
)

if [ -z "$expected_cmds" ]; then
  fail "manpage-completeness: failed to extract any subcommands from $cli_src (parser drift?)"
fi

# clap_mangen renders subcommand references inside the manpage as
# `nixling\-<name>(1)`. We list the actual rendered names, then
# diff against the expected set.
documented_cmds=$(
  awk '
    BEGIN { in_sub = 0 }
    /^\.SH SUBCOMMANDS$/ { in_sub = 1; next }
    in_sub && /^\.SH / { in_sub = 0; next }
    in_sub && /^nixling\\-/ {
      line = $0
      sub(/^nixling\\-/, "", line)
      sub(/\(1\)$/, "", line)
      gsub(/\\-/, "-", line)
      print line
    }
  ' "$manpage" | sort -u
)

if [ -z "$documented_cmds" ]; then
  fail "manpage-completeness: failed to extract any documented subcommands from $manpage (manpage shape drift?)"
fi

missing=$(comm -23 <(printf '%s\n' "$expected_cmds") <(printf '%s\n' "$documented_cmds"))

if [ -n "$missing" ]; then
  echo "manpage-completeness: subcommands declared in $cli_src but missing from $manpage:" >&2
  echo "$missing" | sed 's/^/  - /' >&2
  echo "" >&2
  echo "Regenerate with: cargo xtask gen-cli-shell-artifacts" >&2
  fail "manpage-completeness: $(echo "$missing" | wc -l | tr -d ' ') subcommand(s) missing from manpage"
fi

ok "manpage-completeness: all $(echo "$expected_cmds" | wc -l | tr -d ' ') top-level subcommands documented in docs/manpages/nixling.1"
