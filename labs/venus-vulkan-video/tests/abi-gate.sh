#!/usr/bin/env bash
# abi-gate.sh - enforce the append-only Venus command-ID invariant.
#
# WHY THIS IS A HARD GATE
#
#   Venus requires EXACT VN_WIRE_FORMAT_VERSION equality between guest and
#   renderer, so the version cannot be bumped to signal a protocol change
#   without breaking every existing guest. Video support is therefore added
#   additively, gated by the extension mask instead.
#
#   That is only safe if the serialization ABI is genuinely append-only. If
#   adding extensions to VK_XML_EXTENSION_LIST ever renumbered an existing
#   VK_COMMAND_TYPE_* value, every old guest would break against a new renderer
#   while the version number still claimed compatibility -- a worse failure
#   than an honest version bump, because nothing would announce it.
#
#   Upstream's own utils/print_vk_command_types.py is append-only by
#   construction: it reuses ids already present in the XML and only allocates
#   new ones for genuinely new commands. This gate proves that property held
#   for OUR change rather than assuming the tool behaved.
#
# USAGE
#   abi-gate.sh --snapshot <out.xml>    capture the golden baseline (do this
#                                       from the base/<rev> tag, BEFORE editing)
#   abi-gate.sh --check <golden.xml>    fail if any pre-existing id changed
set -euo pipefail

# Literal newline, for the pattern match in the additivity check below.
nl=$'\n'

VP_DIR="${VENUS_PROTOCOL_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab/forks/venus-protocol}"
PYTHON="${VENUS_LAB_PYTHON:-python3}"

die() { printf 'abi-gate: %s\n' "$*" >&2; exit 1; }

[ -d "$VP_DIR" ] || die "venus-protocol not found at $VP_DIR (set VENUS_PROTOCOL_DIR)"

# Emit "value=<n> name=<VK_COMMAND_TYPE_...>" pairs, whitespace-normalised and
# sorted, so comparison is insensitive to XML formatting and ordering.
emit_ids() {
  "$PYTHON" "$VP_DIR/utils/print_vk_command_types.py" 2>/dev/null \
    | grep -oE 'value="[0-9]+"[[:space:]]+name="VK_COMMAND_TYPE_[A-Za-z0-9_]+"' \
    | sed 's/  */ /g' \
    | sort
}

# Emit "<sha> <symbol>" for every generated serialization function.
#
# Stable command ids are necessary but NOT sufficient. VN_WIRE_FORMAT_VERSION
# stays at 1, so the *layout* of every pre-existing command and struct must also
# be unchanged -- a reordered or resized member would corrupt an old peer just
# as thoroughly as a renumbered command, and just as silently.
#
# Each function body is hashed independently so the comparison reports exactly
# which symbol changed, rather than a whole-file diff.
emit_layout() {
  local outdir
  outdir=$(mktemp -d)
  # shellcheck disable=SC2064
  trap "rm -rf '$outdir'" RETURN

  # Generate the two variants into SEPARATE dirs and tag each symbol with its
  # variant. Many symbols exist in both, so comparing on the bare name makes
  # `join` produce a cross product and report spurious changes.
  mkdir -p "$outdir/renderer" "$outdir/driver"
  "$PYTHON" "$VP_DIR/vn_protocol.py" --renderer --outdir "$outdir/renderer" >/dev/null 2>&1
  "$PYTHON" "$VP_DIR/vn_protocol.py" --outdir "$outdir/driver" >/dev/null 2>&1

  # awk splits the concatenated headers into one record per function definition
  # (a `static inline` block ending at a column-0 closing brace) and prints
  # "<symbol>\t<body>"; sha256sum then reduces each body to a digest.
  #
  # The body is joined with spaces rather than newlines: the reader below is
  # line-oriented, so an embedded newline would turn one function into many
  # bogus records.
  for variant in renderer driver; do
    cat "$outdir/$variant"/*.h 2>/dev/null | awk -v V="$variant" '
      /^vn_(sizeof|encode|decode|replace)_[A-Za-z0-9_]+\(/ {
        sym = $0; sub(/\(.*/, "", sym); collecting = 1; body = ""; next
      }
      collecting && /^}/ {
        gsub(/[ \t]+/, " ", body)
        print V ":" sym "\t" body
        collecting = 0
        next
      }
      collecting { body = body $0 " " }
    '
  done | while IFS=$'\t' read -r sym body; do
      [ -n "$sym" ] || continue
      printf '%s %s\n' "$(printf '%s' "$body" | sha256sum | cut -c1-16)" "$sym"
    done | sort -k2
}

# Emit "<tab><symbol><tab><body>" for every generated serialization function, so
# a changed symbol can be inspected without regenerating the base revision.
emit_bodies() {
  local VP_DIR="${VP_DIR}"
  local outdir
  outdir=$(mktemp -d)
  # shellcheck disable=SC2064
  trap "rm -rf '$outdir'" RETURN

  # Generate the two variants into SEPARATE dirs and tag each symbol with its
  # variant. Many symbols exist in both, so comparing on the bare name makes
  # `join` produce a cross product and report spurious changes.
  mkdir -p "$outdir/renderer" "$outdir/driver"
  "$PYTHON" "$VP_DIR/vn_protocol.py" --renderer --outdir "$outdir/renderer" >/dev/null 2>&1
  "$PYTHON" "$VP_DIR/vn_protocol.py" --outdir "$outdir/driver" >/dev/null 2>&1

  for variant in renderer driver; do
    # Comments are stripped before hashing. They are not wire format, and
    # keeping them makes a legitimate structural transition look like a
    # removal: when a pNext chain gains its FIRST known struct, the generator
    # replaces the degenerate
    #     /* no known/supported struct */ vn_encode_simple_pointer(enc, NULL);
    # with a while/switch whose fall-through path emits exactly those same
    # bytes. The only token that disappears is the comment.
    cat "$outdir/$variant"/*.h 2>/dev/null \
      | sed -e 's:/\*[^*]*\*\+\([^/*][^*]*\*\+\)*/: :g' \
      | awk -v V="$variant" '
      /^vn_(sizeof|encode|decode|replace)_[A-Za-z0-9_]+\(/ {
        sym = $0; sub(/\(.*/, "", sym); collecting = 1; body = ""; next
      }
      collecting && /^}/ {
        gsub(/[ \t]+/, " ", body)
        printf "\t%s:%s\t%s\n", V, sym, body
        collecting = 0
        next
      }
      collecting { body = body $0 " " }
    '
  done
}

case "${1:-}" in
  --snapshot)
    out="${2:?usage: abi-gate.sh --snapshot <out.xml>}"
    mkdir -p "$(dirname "$out")"
    emit_ids > "$out"
    n=$(wc -l < "$out")
    [ "$n" -gt 0 ] || die "snapshot is empty -- did the generator run?"
    echo "abi-gate: captured $n command ids to $out"

    layout_out="${out%.txt}-layout.txt"
    emit_layout > "$layout_out"
    ln=$(wc -l < "$layout_out")
    [ "$ln" -gt 0 ] || die "layout snapshot is empty -- did the generator run?"
    echo "abi-gate: captured $ln serialization layouts to $layout_out"
    ;;

  --check)
    golden="${2:?usage: abi-gate.sh --check <golden.xml>}"
    [ -f "$golden" ] || die "golden snapshot not found: $golden"

    current=$(mktemp); trap 'rm -f "$current"' EXIT
    emit_ids > "$current"

    n_gold=$(wc -l < "$golden")
    n_curr=$(wc -l < "$current")
    echo "abi-gate: golden=$n_gold current=$n_curr"

    # The invariant: every golden entry must still be present, byte-identical.
    # comm -23 lists golden entries absent from current -- i.e. renumbered or
    # removed. Any output at all is a violation.
    removed=$(comm -23 "$golden" "$current" || true)
    if [ -n "$removed" ]; then
      echo "abi-gate: FAIL -- pre-existing command ids changed or removed:" >&2
      printf '%s\n' "$removed" | sed 's/^/  /' >&2
      echo >&2
      echo "  This breaks every existing guest against a new renderer while" >&2
      echo "  VN_WIRE_FORMAT_VERSION still claims compatibility. Assign new" >&2
      echo "  commands the next free id instead of reordering existing ones." >&2
      exit 1
    fi

    added=$(comm -13 "$golden" "$current" || true)
    if [ -z "$added" ]; then
      echo "abi-gate: PASS -- $n_gold ids preserved, no additions"
      exit 0
    fi

    n_added=$(printf '%s\n' "$added" | wc -l)
    min_id=$(printf '%s\n' "$added" | grep -oE 'value="[0-9]+"' | grep -oE '[0-9]+' | sort -n | head -1)
    max_id=$(printf '%s\n' "$added" | grep -oE 'value="[0-9]+"' | grep -oE '[0-9]+' | sort -n | tail -1)
    echo "abi-gate: PASS -- $n_gold ids preserved byte-identical"
    echo "abi-gate: $n_added additions, ids $min_id..$max_id"
    printf '%s\n' "$added" | sed 's/^/  + /'

    # Layout check: every pre-existing serialization function must hash the
    # same. New symbols are fine; changed ones are not.
    layout_gold="${VENUS_ABI_LAYOUT_GOLDEN:-${golden%.txt}-layout.txt}"
    if [ ! -f "$layout_gold" ]; then
      die "no layout snapshot at $layout_gold. This gate is the only thing
that makes keeping VN_WIRE_FORMAT_VERSION at 1 safe, so a missing snapshot
is a hard failure rather than a skip. Regenerate with --snapshot from the
base revision, or point VENUS_ABI_LAYOUT_GOLDEN at the committed copy."
    fi

    layout_cur=$(mktemp); trap 'rm -f "$current" "$layout_cur"' EXIT
    emit_layout > "$layout_cur"
    echo
    echo "abi-gate: layout golden=$(wc -l < "$layout_gold") current=$(wc -l < "$layout_cur")"

    # Compare on symbol name, reporting only symbols whose digest changed.
    changed=$(join -j 2 -o 0,1.1,2.1 \
                <(sort -k2 "$layout_gold") <(sort -k2 "$layout_cur") 2>/dev/null \
              | awk '$2 != $3 { print $1 }')

    if [ -n "$changed" ]; then
      # A changed layout is acceptable ONLY if the change is purely additive:
      # new `case` branches in a pNext dispatch, with no existing line altered
      # or removed.
      #
      # That is the real invariant, and it is weaker than requiring an extension
      # guard. A new pNext case is unreachable for an old peer regardless of
      # guarding, because the sType can only be present if that peer put it in
      # its own chain - the guest builds the chain it sends, and the renderer's
      # reply mirrors it. What would genuinely break compatibility is changing
      # how an EXISTING case encodes, which additivity forbids.
      #
      # Verified by regenerating the base revision and diffing each changed
      # function, rather than allowlisting symbols by name.
      base_dir=$(mktemp -d)
      trap 'rm -f "$current" "$layout_cur"; rm -rf "$base_dir"' EXIT

      if [ -n "${VENUS_PROTOCOL_BASE_DIR:-}" ]; then
        # Supplied by the flake as a pinned store path, which is not a git
        # checkout, so it is staged rather than worktree-added.
        cp -r --no-preserve=mode,ownership "$VENUS_PROTOCOL_BASE_DIR" "$base_dir/src"
      else
        base_rev=$(git -C "$VP_DIR" tag --list 'base/*' 2>/dev/null | head -1)
        [ -n "$base_rev" ] \
          || die "no base revision available: set VENUS_PROTOCOL_BASE_DIR or tag base/*"
        git -C "$VP_DIR" worktree add -q --detach "$base_dir/src" "$base_rev" 2>/dev/null \
          || die "could not check out $base_rev for comparison"
        base_worktree=$base_dir/src
      fi

      mkdir -p "$base_dir/renderer" "$base_dir/driver"
      "$PYTHON" "$base_dir/src/vn_protocol.py" --renderer --outdir "$base_dir/renderer" >/dev/null 2>&1
      "$PYTHON" "$base_dir/src/vn_protocol.py" --outdir "$base_dir/driver" >/dev/null 2>&1

      cur_bodies=$(mktemp); base_bodies=$(mktemp)
      emit_bodies > "$cur_bodies"
      VP_DIR="$base_dir/src" emit_bodies > "$base_bodies"

      # The generator emits a degenerate body for a pNext chain that has no
      # known structs yet. Gaining its FIRST known struct replaces that body
      # with a dispatch loop, which the token check cannot recognise as safe
      # even though it is: the only thing an old peer can place in that slot
      # is the NULL marker, and the dispatching body handles a NULL marker
      # exactly as the degenerate one did.
      #
      # This is recognised explicitly and VERIFIED rather than assumed -- the
      # old body must be one of the three exact degenerate templates, and the
      # new body must still contain the matching NULL-marker path.
      degenerate_pnext_transition() {
        local sym=$1 old=$2 new=$3
        case $sym in *_pnext*) ;; *) return 1 ;; esac
        case $old in
          'return vn_sizeof_simple_pointer(NULL);')
            [ "${new#*"return vn_sizeof_simple_pointer(NULL);"}" != "$new" ] ;;
          'vn_encode_simple_pointer(enc, NULL);')
            [ "${new#*"vn_encode_simple_pointer(enc, NULL);"}" != "$new" ] ;;
          'if (vn_decode_simple_pointer(dec)) vn_cs_decoder_set_fatal(dec); return NULL;')
            [ "${new#*"if (!vn_decode_simple_pointer(dec)) return NULL;"}" != "$new" ] ;;
          *) return 1 ;;
        esac
      }

      nonadditive=""
      while IFS= read -r sym; do
        [ -n "$sym" ] || continue
        old_body=$(grep -F "	$sym	" "$base_bodies" 2>/dev/null | head -1 | cut -f3-)
        new_body=$(grep -F "	$sym	" "$cur_bodies" 2>/dev/null | head -1 | cut -f3-)

        # Drop the opening brace and outer whitespace so the degenerate-template
        # comparison below is exact equality rather than a loose match.
        old_trim=$(printf '%s' "$old_body" | sed 's/^ *{ *//; s/^ *//; s/ *$//')
        new_trim=$(printf '%s' "$new_body" | sed 's/^ *{ *//; s/^ *//; s/ *$//')

        if degenerate_pnext_transition "${sym#*:}" "$old_trim" "$new_trim"; then
          continue
        fi

        # Additive iff no token of the old body was removed or changed --
        # i.e. the old token sequence is still a subsequence of the new one.
        # Compared word-wise so reindentation is ignored.
        #
        # The diff output is captured BEFORE it is searched. Piping straight
        # into `grep -q` here is a false pass: under `set -o pipefail` the
        # pipeline reports diff's non-zero "files differ" status regardless of
        # what grep found, so `! diff ... | grep -q '^<'` takes the safe branch
        # for *every* changed symbol -- including the non-additive ones this
        # gate exists to catch.
        removed=$(diff <(printf '%s\n' "$old_body" | tr ' ' '\n') \
                       <(printf '%s\n' "$new_body" | tr ' ' '\n') || true)
        case $removed in
          *"$nl<"* | "<"*) ;;   # a token was removed or changed -- not additive
          *) continue ;;        # only additions -- safe
        esac
        nonadditive="$nonadditive  ~ $sym"$'\n'
        # VENUS_ABI_DEBUG=1 prints the two bodies for anything the gate
        # rejects. Written as an `if` rather than `[ ... ] && { ... }` so the
        # false case cannot become the loop body's exit status under `set -e`.
        if [ -n "${VENUS_ABI_DEBUG:-}" ]; then
          printf 'DBG %s\n  old=[%s]\n  new=[%s]\n' \
            "$sym" "$old_trim" "${new_trim:0:160}" >&2
        fi
      done <<< "$changed"

      [ -n "${base_worktree:-}" ] \
        && git -C "$VP_DIR" worktree remove --force "$base_worktree" 2>/dev/null
      base_worktree=""
      rm -f "$cur_bodies" "$base_bodies"

      if [ -n "$nonadditive" ]; then
        echo "abi-gate: FAIL -- serialization layout changed NON-additively:" >&2
        printf '%s' "$nonadditive" | head -20 >&2
        echo >&2
        echo "  VN_WIRE_FORMAT_VERSION is still 1, so an existing peer decodes" >&2
        echo "  these with the old layout. Only additive changes are safe: a new" >&2
        echo "  case an old peer never reaches. Altering an existing case changes" >&2
        echo "  the meaning of bytes that peer is already sending." >&2
        exit 1
      fi

      n_changed=$(printf '%s\n' "$changed" | grep -c . || true)
      echo "abi-gate: $n_changed layout(s) changed, all verified purely additive"
      printf '%s\n' "$changed" | sed 's/^/  ~ /' | head -12
    fi

    gone=$(comm -23 <(cut -d' ' -f2 "$layout_gold" | sort) \
                    <(cut -d' ' -f2 "$layout_cur" | sort) || true)
    if [ -n "$gone" ]; then
      echo "abi-gate: FAIL -- serialization functions removed:" >&2
      printf '%s\n' "$gone" | sed 's/^/  - /' | head -20 >&2
      exit 1
    fi

    new_syms=$(comm -13 <(cut -d' ' -f2 "$layout_gold" | sort) \
                        <(cut -d' ' -f2 "$layout_cur" | sort) | wc -l)
    echo "abi-gate: PASS -- all pre-existing serialization layouts unchanged"
    echo "abi-gate: $new_syms new serialization symbols"
    ;;

  -h|--help) sed -n '2,26p' "$0" ;;
  *) die "usage: abi-gate.sh --snapshot <out> | --check <golden>" ;;
esac
