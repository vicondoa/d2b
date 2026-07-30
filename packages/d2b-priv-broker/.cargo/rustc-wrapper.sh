#!/usr/bin/env sh
# Resilient rustc wrapper: use sccache when it is available, otherwise run
# rustc directly.
#
# Pointing [build] rustc-wrapper straight at "sccache" makes the binary a hard
# requirement of every cargo invocation, so any environment without it - a
# focused `nix shell ... cargo test`, a CI job that caches target dirs instead,
# a nix sandbox build - fails outright. The workaround people reach for is to
# export RUSTC_WRAPPER="" , and that override then gets copied into
# environments that DO have sccache, silently disabling the cache and making
# builds much slower for no reason.
#
# Routing through this shim removes the reason to ever clear the variable.
#
# When sccache itself fails - it cannot reach or start its background server,
# it crashes, a protocol error - that is not a compiler diagnostic and must not
# read like one. Passing sccache's exit code straight through would make "the
# cache broke" indistinguishable from "the code does not compile", and the
# fix for the two is opposite (restart/bypass sccache vs. fix the code). Detect
# a known sccache-failure signature and exit 97 with an explicit message
# instead, so a wrapper failure is never mistaken for a compile failure.
set -eu

if ! command -v sccache >/dev/null 2>&1; then
  exec "$@"
fi

stderr_capture=$(mktemp "${TMPDIR:-/tmp}/d2b-rustc-wrapper-stderr.XXXXXX")
trap 'rm -f "$stderr_capture"' EXIT

status=0
sccache "$@" 2>"$stderr_capture" || status=$?
cat "$stderr_capture" >&2

if [ "$status" -ne 0 ] && grep -qE \
  'sccache: (error|encountered)|failed to (start|spawn|connect to) (the )?(sccache )?server|couldn.t connect to (the )?server|Connection refused|Broken pipe|panicked at' \
  "$stderr_capture"; then
  printf 'd2b-rustc-wrapper: sccache failed to invoke the compiler (wrapper error, not a compile error); original exit %s\n' "$status" >&2
  exit 97
fi

exit "$status"
