### Changed

- Froze the canonical per-Zone bundle digest chain (D119) as four fully
  computable members: the bundle `contentHash` over the canonical sorted
  `resources` array (which also serves as the generation identity and the
  preimage covering every bundled envelope), `providerSchemaDigests`, the
  artifact-catalog document digest carried as `catalogDigest` and anchored in
  each bundle's Nix-store-immutable `artifactCatalogDigest`, and per-artifact
  store-path hashes verified at apply. Removed the unpinned self-digest claim
  that could not detect tampering, and named the store-side
  `d2b:v3:resource-envelope` tag as explicitly outside the bundle chain.
- Made provider-catalog identity decidable (D120): `spec.artifactId` must be
  unique across `d2b.providerCatalog` entries, enforced by an eval-time
  assertion (`provider-catalog-duplicate-artifact-id`), so "resolves exactly one
  entry" is enforceable rather than relying on attrset key uniqueness.
- Assigned a single durable writer for configuration activation (D122):
  `generation.json` (active pointer, prior pointer, retention metadata) is
  committed in one atomic durable write before any diff application or
  reconcile notification, and every other activation work item defers that
  commit to the sole writer. Restart recovery follows ADR 0034: recover, adopt,
  or quarantine before any cleanup.
- Redefined the host-firewall projection generation fence (D125) as the
  immutable installed configuration generation (`expected_generation_id`),
  removing the unimplementable live projection counter and its
  compare-and-advance; concurrent same-projection mutations serialize on the
  ordered OFD lock over the `inet d2b` table and converge idempotently.
- Froze the ZoneLink enrollment and key lifecycle (D126): the one-time IKpsk2
  bootstrap session is terminated after enrollment and never rekeyed or
  continued, a distinct enrolled `Noise_KK` handshake from a durable
  `EnrollmentCommitted` state must complete before `Ready`, and the bootstrap
  PSK TTL, KK cryptoperiod, and every authentication-failure transition are now
  frozen protocol constants with no fallback below the enrolled KK contract
  short of durable revocation.
- Forbade cross-Zone L2 sharing on multiplexed external physical NICs (D127):
  the Host-global external physical-NIC authority binds an isolation domain
  equal to the claimant Zone UID, `bridge` multiplexing is admitted only among
  same-Zone claimants, and a cross-Zone `bridge` multiplex is rejected fail
  closed (`external-physical-nic-cross-zone-l2`) so work and personal Zones
  never share an L2 broadcast domain.
- Rewrote host cutover as in-place adaptation of exactly the three root-visible
  units (`d2bd.service`, `d2b-priv-broker.socket`, `d2b-priv-broker.service`):
  removed the parallel Zone-runtime unit set and the step that destroyed the
  three units, and required an exact-three-units integration assertion matching
  the framework host exit criterion.

### Fixed

- Propagated D119 across every spec that still froze retired bundle names,
  replacing `contentId`, numeric bundle generations, `resources.json`,
  `bundleSha256`, `catalogSha256`, `BundleManifest`, and
  `retainedConfigurationMax` with the D119 `contentHash`,
  `resource-bundle.json`, no-manifest, and sole-`retainedGenerations` contract,
  and corrected D119's affected-specs column to list every reconciled spec.
- Corrected the decision register to stop describing completed work as pending:
  D121 no longer says the host/guest/process spec "must adopt"
  `backoffMultiplierMilli` (already adopted), and the resolved-decision and
  cross-provider sections of the network spec no longer cite the wrong register
  ID or mark USBIP firewall reconciliation as pending.
- Mapped USBIP apply and release firewall intent onto the shared
  `ApplyNftablesProjection` op (D124) across all sibling cells in the network
  spec, replacing the fictional `UsbipBindFirewallRule { action: Ensure |
  Remove }` shape; USBIP firewall release is documented as net-new privileged
  surface rather than an existing action.
- Required the Azure relay transport dossier to terminate the IKpsk2 bootstrap
  session after enrollment and complete a distinct enrolled KK handshake before
  `Ready`, in place of rekeying the bootstrap session into steady state, and
  added a validation case rejecting continuation or resource traffic on
  IKpsk2-derived state.
- Removed the systemd path unit from configuration bundle watching in the
  provider-state spec; the Zone daemon watches the installed bundle in-process
  and is signalled through the activation protocol, preserving the
  three-root-visible-unit contract.
