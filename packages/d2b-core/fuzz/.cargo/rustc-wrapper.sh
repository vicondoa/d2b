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
set -eu

if command -v sccache >/dev/null 2>&1; then
  exec sccache "$@"
fi

exec "$@"
