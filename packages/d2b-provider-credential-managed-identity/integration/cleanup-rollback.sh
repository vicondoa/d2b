#!/usr/bin/env bash
set -euo pipefail

# integration-target: host-integration
# Agent drain, Process deletion, finalizer release, and rollback require a
# booted NixOS Zone runtime. Refuse to report hermetic success without it.
harness=${D2B_CREDENTIAL_PROVIDER_HOST_HARNESS:-}
if [[ -z "$harness" ]]; then
  printf '%s\n' 'managed-identity cleanup requires the host Zone harness' >&2
  exit 77
fi
exec "$harness" credential-managed-identity cleanup-rollback
