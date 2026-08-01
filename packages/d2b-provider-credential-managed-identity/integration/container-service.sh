#!/usr/bin/env bash
set -euo pipefail

# integration-target: container
# The container lane supplies fake IMDS through an injected effect port. No
# test in this crate contacts Azure or a live metadata endpoint.
harness=${D2B_CREDENTIAL_PROVIDER_CONTAINER_HARNESS:-}
if [[ -z "$harness" ]]; then
  printf '%s\n' 'managed-identity container scenario requires the fake IMDS Zone harness' >&2
  exit 77
fi
exec "$harness" credential-managed-identity controller-agent-lifecycle
