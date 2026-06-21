# Migrate to explicit realm gateways

**Diataxis category:** how-to.

This guide is version-neutral until the release boundary for the realm rollout
is finalized. If the release introduces a versioned migration guide, this file
should be renamed to the repository's version-bound migration convention.

## What changes

- Bare VM names stay local.
- The reserved `local` realm stays host-resident.
- Work, provider, and cross-host realms use explicit gateway-backed
  entrypoints.
- Cross-realm operations and streams are denied by default.
- Relay/provider credentials are enrolled inside the gateway guest, not stored
  or minted by the host.

## Migration steps

1. Declare a dedicated `nixling.envs.<realm>` for each trust-boundary realm.
2. Declare one `nixling.gateways.<name>` per gateway-backed realm.
3. Rebuild the host and inspect the rendered policy with
   `nixling realm list` and `nixling realm inspect <realm>`.
4. Start each gateway VM and enroll credentials from inside the gateway guest.
5. Update scripts to use fully-qualified realm targets only where remote or
   provider routing is intended.

If an existing deployment only uses local VMs and no gateway credentials, no
state migration is required: local VM names and the `local` realm continue to
use the host fast path.

## Verify isolation

Confirm that work and personal/provider realms do not share gateway guests,
envs, or L2 bridges. Stopped gateways must fail closed with remediation rather
than falling back to host credentials, SSH, or host routing.
