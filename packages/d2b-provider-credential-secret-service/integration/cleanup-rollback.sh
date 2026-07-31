#!/usr/bin/env bash
set -euo pipefail

# integration-target: host-integration
# Generation removal and rollback require a booted NixOS host and the live
# resource finalizer path. Refuse to report hermetic success without it.
harness=${D2B_CREDENTIAL_PROVIDER_HOST_HARNESS:-}
if [[ -z "$harness" ]]; then
  printf '%s\n' 'credential-secret-service cleanup requires the host Zone harness' >&2
  exit 77
fi
exec "$harness" credential-secret-service cleanup-rollback
