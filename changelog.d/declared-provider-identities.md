### Fixed

- A crate's provider identity is no longer inferred from its directory name.
  The registration and service-catalog authorities composed
  `Provider/<directory-suffix>` whenever a crate carried no declaration, which
  fabricated an identity for every provider crate that had not declared one and
  published five of them wrong: `d2b-provider-process-minijail` published
  `Provider/process-minijail` for a family that is `system-minijail`. The
  identity is now the declaration states it or the row is null; nothing composes
  one.
- A crate can declare the identity it actually registers. The parity gates held
  a declared identity to the suffix of the directory beside it, so a crate whose
  directory does not spell its identity could not state it at all - the check
  compared each declaration against the very string the old inference would have
  produced. Both gates now hold the identity to the resource-name grammar the
  contracts admit, which is the whole of what a declared identity owes, and
  uniqueness across crates is still gated.
- The generated registration table carries the identities eleven families were
  already named by in production but no declaration stated:
  `credential-entra`, `credential-managed-identity`, `credential-secret-service`,
  `device-gpu`, `guest-azure-container-apps` as
  `runtime-azure-container-apps`, `guest-azure-virtual-machine` as
  `runtime-azure-virtual-machine`, `guest-cloud-hypervisor` as
  `runtime-cloud-hypervisor`, `guest-qemu-media` as `runtime-qemu-media`,
  `transport-azure-relay`, `volume-local`, and `volume-virtiofs`. Each is named
  by production sources outside the provider crate graph - the NixOS assertion
  and module surfaces that gate the family's configuration, the resource
  contract constants that gate its specs, and the daemon fences that admit its
  rows - so the row is now a statement a reviewer can check rather than a null.
- The remaining provider crates state no identity on purpose, and say why in the
  shared absence table. A crate registers no identity unless a production source
  outside the crate graph names it, so the rows that stay null are reviewable
  decisions with the naming source recorded next to them.

### Changed

- `d2b-provider-device-tpm` still registers no identity, and the absence table
  now says why rather than leaving the reason to be guessed. Its identity is
  named by production sources like every other declared family, but the crate
  spells a `ServiceDecl` its own registration would have to carry, and the
  daemon's production composition has no factory that hosts it: declaring the
  service fails the declaration-to-source parity gate, and declaring no service
  leaves a published service out of the registration table. Both forms would
  state something untrue, so the null is the honest row until the composition
  hosts the family.
