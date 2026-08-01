#!/usr/bin/env bash
set -euo pipefail

# integration-target: container
# Live cloud access is forbidden. The container lane supplies a fake Entrablau
# Endpoint and authenticated cross-process Zone routing.
harness=${D2B_CREDENTIAL_PROVIDER_CONTAINER_HARNESS:-}
if [[ -z "$harness" ]]; then
  printf '%s\n' 'credential-entra container scenario requires the fake Entrablau Zone harness' >&2
  exit 77
fi
exec "$harness" credential-entra service-lifecycle
