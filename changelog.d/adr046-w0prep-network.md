### Fixed

- Closed the ZoneLink enrollment-bootstrap bypass across every
  implementation-driving surface in `docs/specs/**`, not just the canonical
  state machine. The controller algorithm in
  `ADR-046-resources-zone-control.md`, the ZoneLink resource work item
  (`ADR046-zone-control-002`) and its validation cells, and the
  `transport-unix`, `transport-vsock`, and `transport-azure-relay` provider
  dossiers now all state the exact sequence
  `Unenrolled -> IKpsk2 -> EnrollmentCommitted -> KK -> Ready`: the one-time
  IKpsk2 bootstrap consuming the allocator-issued single-use PSK runs only from
  `Unenrolled` (and after revocation), reconnect re-enters at `KK` from
  `EnrollmentCommitted` without a PSK, and resource-plane traffic is prohibited
  until `Ready`. Every transport sequence description now states explicitly
  that the selected transport Provider never selects, negotiates, or reorders
  handshake profiles, so no implementation-driving surface permits the
  steady-state-only KK-direct downgrade.

### Changed

- Redesigned the per-Network host-firewall model so it no longer maps each
  dynamic per-Network `FirewallIntent` onto the whole-table `ApplyNftables`
  broker request. Because the shipped `ApplyNftables` op discards its
  `ownership_id` and atomically deletes and recreates the entire `inet d2b`
  table, mapping per-UID Network reconciles (and per-UID deletion) onto it
  made independent Network projections last-writer-wins and erased other
  Networks' rules and the device-usbip firewall rules. The `Provider/network-local`
  and `ADR-046-resources-network.md` specs now define a new closed broker
  operation, `ApplyNftablesProjection`, that atomically applies or removes
  exactly one validated, generation-fenced ownership projection resolved from
  the private bundle, byte-preserves every other Network and USBIP ownership
  marker, fails closed on a foreign marker (`foreign-nft-rule-preserved`), and
  returns a projection-scoped digest. Decision D-NETWORK-004 records the
  rationale and the cross-provider invariant that any provider mutating the
  `inet d2b` table must use a projection-scoped op.
