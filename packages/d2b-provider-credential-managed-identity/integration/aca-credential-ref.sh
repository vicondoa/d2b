#!/usr/bin/env bash
set -euo pipefail

# integration-target: container
# Validate a rendered ACA Provider artifact supplied by the container lane.
artifact=${D2B_ACA_PROVIDER_CONFIG:-}
if [[ -z "$artifact" || ! -f "$artifact" ]]; then
  printf '%s\n' 'ACA credentialRef validation requires D2B_ACA_PROVIDER_CONFIG' >&2
  exit 77
fi
if command -v jq >/dev/null 2>&1; then
  jq -e '.credentialRef == "Credential/aca-relay-mi"' "$artifact" >/dev/null
else
  printf '%s\n' 'ACA credentialRef validation requires jq' >&2
  exit 77
fi
if LC_ALL=C command grep -q 'managed_identity_client_id' "$artifact"; then
  printf '%s\n' 'raw managed identity client ID field remains in ACA config' >&2
  exit 1
fi
