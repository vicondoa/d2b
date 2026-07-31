#!/usr/bin/env bash
set -euo pipefail

# integration-target: container
# The container lane must supply the cross-process Zone harness. The hermetic
# crate cannot claim D-Bus, process lifecycle, or drain coverage.
harness=${D2B_CREDENTIAL_PROVIDER_CONTAINER_HARNESS:-}
if [[ -z "$harness" ]]; then
  printf '%s\n' 'credential-secret-service container scenario requires the container Zone harness' >&2
  exit 77
fi
exec "$harness" credential-secret-service service-lifecycle
