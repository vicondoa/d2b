#!/usr/bin/env bash
# tests/video-sidecar-hardening-eval.sh — DEFERRED after daemon-only cutover.
#
# Before the daemon-only cutover, this gate locked in the systemd-unit hardening of the
# per-VM `nixling-<vm>-video.service` (RestrictAddressFamilies =
# [AF_UNIX], RestrictNamespaces=true, SystemCallFilter pinning,
# empty Capability{BoundingSet,Ambient}).
#
# Deletes `nixos-modules/components/video/host.nix` along with
# every `nixling-<vm>-video` systemd unit. The video sidecar is now
# spawned by the nixling priv-broker as `SpawnRunner{role: Video}`;
# the broker applies the equivalent hardening at fork time via its
# canonical cap-set + seccomp profile (see
# `packages/nixling-priv-broker/src/runners/video.rs` and the
# broker-caps-eval gate).
#
# Eval-time assertion of the surviving runner-side hardening is
# being added in a follow-up (broker-video-hardening-eval); until
# then this gate is a no-op stub that the eval-loop can keep
# invoking without failing.

log() { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
log "==> tests/video-sidecar-hardening-eval.sh (stub; surface moved to broker SpawnRunner{Video} in ph6-remove-systemd-emission)"
exit 0
