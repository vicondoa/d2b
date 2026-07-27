### Fixed

- Corrected the ADR 0046 USBIP firewall specs, which described a broker
  capability that does not exist. The shipped `UsbipBindFirewallRule` op is
  bind-only: it carries a single bundle intent reference, has no `action` field
  and no release path, and routes through the whole-table
  `render_owned_table_replace_script`, which deletes and recreates the entire
  `inet d2b` table. The `device-usbip` provider dossier, the security and
  threat model, and the device resources reference nevertheless claimed a closed
  `UsbipBindFirewallRule { action: Ensure | Remove }` op served both acquisition
  and release with "no separate release operation" and "no new privileged
  surface". The most dangerous of these, in
  `ADR-046-security-and-threat-model.md`, asserted USBIP release was the
  existing op "with closed `action: Remove`, not a renamed or second release
  variant"; that would have told an implementer no new privileged surface was
  needed. Every such statement is now corrected to describe the shipped op
  honestly and to state plainly that USBIP firewall release is net-new
  privileged surface.

### Changed

- Aligned the ADR 0046 USBIP host-firewall model onto the same closed
  `ApplyNftablesProjection` broker op that `Provider/network-local` uses, rather
  than the shipped whole-table `UsbipBindFirewallRule` op. Because the shipped op
  replaces the entire `inet d2b` table, an independent USBIP reconcile through it
  would erase Network-owned rules, violating the ownership-marker preservation
  contract (`foreign-nft-rule-preserved`). The `device-usbip` provider now maps
  `apply_firewall`/`release_firewall` onto `ApplyNftablesProjection` with actions
  `Apply|Remove`, resolving the per-Network/per-busid ownership projection from
  the integrity-pinned private bundle, fencing on a projection generation,
  treating a validated already-absent projection as idempotent success,
  byte-preserving every sibling network-local and device-usbip marker, failing
  closed on a foreign marker, and returning a projection-scoped digest. This
  conforms USBIP to decision D-NETWORK-004, whose cross-provider invariant
  requires any provider mutating the `inet d2b` table to use a projection-scoped
  op. New validation cells cover concurrent USBIP apply, concurrent independent
  release, and preservation of a network-local marker across USBIP apply and
  release.
