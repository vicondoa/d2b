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
#
# The signature must be anchored to sccache's OWN output. Matching bare text
# like "panicked at" or "Broken pipe" anywhere in stderr misclassifies in the
# more dangerous direction: a rustc internal compiler error prints "thread
# 'rustc' panicked at" and a rendered diagnostic echoes the offending source
# line, and this repository has source containing both of those strings. Either
# would report a genuine build failure as a cache problem and send someone to
# restart sccache while the compiler was telling them about a real error.
set -eu

if ! command -v sccache >/dev/null 2>&1; then
  exec "$@"
fi

stderr_capture=$(mktemp "${TMPDIR:-/tmp}/d2b-rustc-wrapper-stderr.XXXXXX")
# Flush the capture before removing it on a signal: cargo signals its rustc
# children on Ctrl-C, on first-error abort and on job timeout, and an EXIT-only
# trap would both leak the file and swallow whatever diagnostics had been
# written before the interruption.
trap 'rm -f "$stderr_capture"' EXIT
trap 'cat "$stderr_capture" >&2 2>/dev/null || true; rm -f "$stderr_capture"; exit 130' INT
trap 'cat "$stderr_capture" >&2 2>/dev/null || true; rm -f "$stderr_capture"; exit 143' TERM HUP

status=0
sccache "$@" 2>"$stderr_capture" || status=$?
cat "$stderr_capture" >&2

if [ "$status" -ne 0 ] \
  && grep -qE '^sccache: (error|encountered|Failed)|sccache::|failed to (start|spawn|connect to) (the )?sccache' "$stderr_capture" \
  && ! grep -qE "thread '"'rustc'"' panicked|internal compiler error" "$stderr_capture"; then
  printf 'd2b-rustc-wrapper: sccache failed to invoke the compiler (wrapper error, not a compile error); original exit %s\n' "$status" >&2
  exit 97
fi

exit "$status"
