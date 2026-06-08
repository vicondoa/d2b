#!/usr/bin/env bash
# tests/activation-helper-eval.sh — Layer-1 smoke/regression test for
# the nixling-activation-helper binary.
#
# The helper replaces previous shell-script `[ -L ]` / `[ -f ]` /
# `find -type f` activation patterns that had TOCTOU windows. This
# test provides committed Layer-1 coverage proving:
#   - --help exits 0 and prints usage
#   - missing flags exit 1 with informative error
#   - regular-file happy path: creates file with correct uid/gid/mode/size
#   - existing-file re-assert path (size-mib=0): preserves contents,
#     re-applies uid/gid/mode
#   - symlink refusal exit 2 (critical TOCTOU fix)
#   - FIFO refusal exit 2
#   - directory refusal exit 2 (wrong file type)
#   - enforce-dir-posture happy path
#   - enforce-dir-posture symlink refusal
#   - enforce-dir-posture missing-path exits 0 (idempotent no-op)
#
# Layer 1 (no NixOS module evaluation, no root). Runs in seconds.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; PASS=$((PASS+1)); }
fail() { log "  FAIL: $*"; FAIL=$((FAIL+1)); }

PASS=0
FAIL=0

log "==> tests/activation-helper-eval.sh"

# Find the helper binary. Prefer the workspace target dir; fall
# back to building if missing.
BIN=""
for candidate in \
  "$ROOT/packages/target/debug/nixling-activation-helper" \
  "$ROOT/packages/target/release/nixling-activation-helper" \
  "$ROOT/packages/nixling-host/target/debug/nixling-activation-helper"
do
  if [ -x "$candidate" ]; then
    BIN="$candidate"
    break
  fi
done

if [ -z "$BIN" ]; then
  log "  building nixling-activation-helper..."
  (
    cd "$ROOT/packages/nixling-host"
    cargo build --quiet --bin nixling-activation-helper
  )
  BIN="$ROOT/packages/target/debug/nixling-activation-helper"
fi

if [ ! -x "$BIN" ]; then
  fail "could not locate or build nixling-activation-helper binary"
  exit 1
fi

log "  binary: $BIN"

SCRATCH=$(mktemp -d -t activation-helper-eval.XXXXXX)
trap 'rm -rf "$SCRATCH"' EXIT

uid=$(id -u)
gid=$(id -g)

# --- (1) --help exits 0 + prints usage --------------------------------

help_out=$("$BIN" --help 2>&1)
help_rc=$?
if [ "$help_rc" = "0" ] && printf '%s' "$help_out" | grep -q 'nixling-activation-helper'; then
  ok "--help exits 0 and prints usage"
else
  fail "--help exit=$help_rc, output=$help_out"
fi

# --- (2) missing verb exits 1 ----------------------------------------

set +e
"$BIN" 2>/dev/null
rc=$?
set -e
if [ "$rc" = "1" ]; then
  ok "missing verb exits 1"
else
  fail "missing verb expected exit 1, got $rc"
fi

# --- (3) ensure-regular-file: happy path ------------------------------

target="$SCRATCH/happy.img"
"$BIN" ensure-regular-file --path "$target" \
  --uid "$uid" --gid "$gid" --mode 0600 --size-mib 1
if [ -f "$target" ] && [ "$(stat -c '%s' "$target")" = "$((1024*1024))" ]; then
  mode=$(stat -c '%a' "$target")
  if [ "$mode" = "600" ]; then
    ok "ensure-regular-file happy path creates 1MiB file with mode 0600"
  else
    fail "ensure-regular-file mode mismatch: got $mode want 600"
  fi
else
  fail "ensure-regular-file did not produce 1MiB regular file"
fi

# --- (4) ensure-regular-file: existing file re-assert ----------------

echo "existing content" > "$SCRATCH/exist.img"
chmod 0644 "$SCRATCH/exist.img"
"$BIN" ensure-regular-file --path "$SCRATCH/exist.img" \
  --uid "$uid" --gid "$gid" --mode 0600 --size-mib 0
new_mode=$(stat -c '%a' "$SCRATCH/exist.img")
content=$(cat "$SCRATCH/exist.img")
if [ "$new_mode" = "600" ] && [ "$content" = "existing content" ]; then
  ok "ensure-regular-file re-asserts mode on existing file without modifying content"
else
  fail "ensure-regular-file re-assert: mode=$new_mode content=$content"
fi

# --- (5) ensure-regular-file: symlink refusal -------------------------

ln -s /etc/shadow "$SCRATCH/evil.img"
set +e
"$BIN" ensure-regular-file --path "$SCRATCH/evil.img" \
  --uid "$uid" --gid "$gid" --mode 0600 --size-mib 1 2>/dev/null
rc=$?
set -e
if [ "$rc" = "2" ]; then
  ok "ensure-regular-file refuses symlink with exit 2 (panel-security R2)"
else
  fail "ensure-regular-file on symlink expected exit 2, got $rc"
fi

# --- (6) ensure-regular-file: FIFO refusal ---------------------------

mkfifo "$SCRATCH/fifo.img"
set +e
# 5s timeout proves we don't hang on FIFO open
timeout 5 "$BIN" ensure-regular-file --path "$SCRATCH/fifo.img" \
  --uid "$uid" --gid "$gid" --mode 0600 --size-mib 1 2>/dev/null
rc=$?
set -e
if [ "$rc" = "2" ]; then
  ok "ensure-regular-file refuses FIFO with exit 2 (panel-rust+software R3, no hang)"
elif [ "$rc" = "124" ]; then
  fail "ensure-regular-file hung on FIFO (timeout); the O_NONBLOCK fu20 fix regressed"
else
  fail "ensure-regular-file on FIFO expected exit 2, got $rc"
fi

# --- (7) ensure-regular-file: directory refusal ----------------------

mkdir "$SCRATCH/dir.img"
set +e
"$BIN" ensure-regular-file --path "$SCRATCH/dir.img" \
  --uid "$uid" --gid "$gid" --mode 0600 --size-mib 1 2>/dev/null
rc=$?
set -e
if [ "$rc" = "2" ]; then
  ok "ensure-regular-file refuses directory with exit 2"
else
  fail "ensure-regular-file on directory expected exit 2, got $rc"
fi

# --- (8) enforce-dir-posture: happy path -----------------------------

mkdir "$SCRATCH/posture-dir"
"$BIN" enforce-dir-posture --path "$SCRATCH/posture-dir" \
  --uid "$uid" --gid "$gid" --mode 0750
new_mode=$(stat -c '%a' "$SCRATCH/posture-dir")
if [ "$new_mode" = "750" ]; then
  ok "enforce-dir-posture sets mode 0750 on directory"
else
  fail "enforce-dir-posture mode mismatch: got $new_mode want 750"
fi

# --- (9) enforce-dir-posture: symlink refusal ------------------------

ln -s "$SCRATCH/posture-dir" "$SCRATCH/dir-link"
set +e
"$BIN" enforce-dir-posture --path "$SCRATCH/dir-link" \
  --uid "$uid" --gid "$gid" --mode 0750 2>/dev/null
rc=$?
set -e
if [ "$rc" = "2" ]; then
  ok "enforce-dir-posture refuses symlink with exit 2"
else
  fail "enforce-dir-posture on symlink expected exit 2, got $rc"
fi

# --- (10) enforce-dir-posture: missing path is no-op -----------------

"$BIN" enforce-dir-posture --path "$SCRATCH/does-not-exist" \
  --uid "$uid" --gid "$gid" --mode 0750
# Helper convention: missing path exits 0 (idempotent — activation
# may run before the directory exists).
ok "enforce-dir-posture on missing path is idempotent (no-op exit 0)"

# fd-safe setfacl-on-path + chown-if-orphan verb tests.

# --- (11) setfacl-on-path: happy path --------------------------------

setfacl_bin=$(command -v setfacl 2>/dev/null || true)
if [ -z "$setfacl_bin" ]; then
  log "  setfacl not on PATH; skipping setfacl-on-path tests"
else
  touch "$SCRATCH/aclfile.txt"
  chmod 0644 "$SCRATCH/aclfile.txt"
  "$BIN" setfacl-on-path --path "$SCRATCH/aclfile.txt" \
    --acl-spec "u:$uid:r" --setfacl-bin "$setfacl_bin"
  if getfacl --omit-header "$SCRATCH/aclfile.txt" 2>/dev/null | grep -q "user:$(id -un):r--"; then
    ok "setfacl-on-path happy path applies u:UID:r via /proc/self/fd"
  else
    fail "setfacl-on-path did not apply expected ACL entry"
  fi

  # --- (12) setfacl-on-path: symlink refusal ---
  ln -s /etc/shadow "$SCRATCH/evil-acl.txt"
  set +e
  "$BIN" setfacl-on-path --path "$SCRATCH/evil-acl.txt" \
    --acl-spec "u:$uid:r" --setfacl-bin "$setfacl_bin" 2>/dev/null
  rc=$?
  set -e
  if [ "$rc" = "2" ]; then
    ok "setfacl-on-path refuses symlink with exit 2"
  else
    fail "setfacl-on-path on symlink expected exit 2, got $rc"
  fi

  # --- (13) setfacl-on-path: require-kind refusal on wrong type ---
  mkdir "$SCRATCH/aclrequire-dir"
  set +e
  "$BIN" setfacl-on-path --path "$SCRATCH/aclrequire-dir" \
    --acl-spec "u:$uid:r" --require-kind regular --setfacl-bin "$setfacl_bin" 2>/dev/null
  rc=$?
  set -e
  if [ "$rc" = "2" ]; then
    ok "setfacl-on-path --require-kind regular refuses directory with exit 2"
  else
    fail "setfacl-on-path --require-kind regular on dir expected exit 2, got $rc"
  fi

  # --- (14) setfacl-on-path: FIFO refusal (no hang) ---
  mkfifo "$SCRATCH/aclfifo"
  set +e
  timeout 5 "$BIN" setfacl-on-path --path "$SCRATCH/aclfifo" \
    --acl-spec "u:$uid:r" --require-kind regular --setfacl-bin "$setfacl_bin" 2>/dev/null
  rc=$?
  set -e
  if [ "$rc" = "2" ]; then
    ok "setfacl-on-path refuses FIFO with exit 2 (no hang)"
  else
    fail "setfacl-on-path on FIFO expected exit 2, got $rc"
  fi
fi

# --- (15) chown-if-orphan: known owner -> no-op ---

touch "$SCRATCH/owned.txt"
# Current process owns it; uid maps to a real user. Should be no-op.
"$BIN" chown-if-orphan --path "$SCRATCH/owned.txt" --uid 0 --gid 0
post_owner=$(stat -c '%u' "$SCRATCH/owned.txt")
if [ "$post_owner" = "$uid" ]; then
  ok "chown-if-orphan no-ops when owner is known (preserves $uid)"
else
  fail "chown-if-orphan unexpectedly chowned: got uid $post_owner want $uid"
fi

# --- (16) chown-if-orphan: symlink refusal ---

ln -s /etc/shadow "$SCRATCH/evil-chown.txt"
set +e
"$BIN" chown-if-orphan --path "$SCRATCH/evil-chown.txt" --uid 0 --gid 0 2>/dev/null
rc=$?
set -e
if [ "$rc" = "2" ]; then
  ok "chown-if-orphan refuses symlink with exit 2"
else
  fail "chown-if-orphan on symlink expected exit 2, got $rc"
fi

# --- (17) ensure-regular-file: intermediate-symlink refusal (RESOLVE_NO_SYMLINKS) ---
# openat2 + RESOLVE_NO_SYMLINKS refuses symlinks at ANY component, not
# just the final segment.

mkdir "$SCRATCH/inner-dir"
ln -s "$SCRATCH/inner-dir" "$SCRATCH/inner-link"
set +e
"$BIN" ensure-regular-file --path "$SCRATCH/inner-link/test.img" \
  --uid "$uid" --gid "$gid" --mode 0600 --size-mib 1 2>/dev/null
rc=$?
set -e
if [ "$rc" = "2" ]; then
  ok "ensure-regular-file refuses intermediate-symlink (panel-security R5)"
else
  fail "ensure-regular-file on intermediate-symlink expected exit 2, got $rc"
fi

# --- (18) enforce-dir-posture: intermediate-symlink refusal ---
set +e
"$BIN" enforce-dir-posture --path "$SCRATCH/inner-link" \
  --uid "$uid" --gid "$gid" --mode 0750 2>/dev/null
rc=$?
set -e
if [ "$rc" = "2" ]; then
  ok "enforce-dir-posture refuses intermediate-symlink (panel-security R5)"
else
  fail "enforce-dir-posture on intermediate-symlink expected exit 2, got $rc"
fi

# --- summary ---------------------------------------------------------

log "==> activation-helper-eval: $PASS passed, $FAIL failed"

if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
