#!/usr/bin/env bash
set -euo pipefail

# integration-target: host-integration
# Credential cleanup must preserve identity Guest TPM and login state. That
# requires the booted Guest and resource finalizer path.
harness=${D2B_CREDENTIAL_PROVIDER_HOST_HARNESS:-}
if [[ -z "$harness" ]]; then
  printf '%s\n' 'credential-entra cleanup requires the host Zone harness' >&2
  exit 77
fi
exec "$harness" credential-entra cleanup-rollback
