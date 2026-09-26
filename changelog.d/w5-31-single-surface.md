### Changed

- Sixteen crates no longer name the same public item at two paths: where a
  module was both `pub mod` and re-exported at the crate root, exactly one arm
  survives. The modules consumers reach through their module path keep it and
  drop the redundant root re-export, with the former root-path callers moved
  onto the module path: the `effects_service` module of
  `d2b-provider-device`, `d2b-provider-device-usbip` and
  `d2b-provider-device-security-key`, `d2b-provider-endpoint`'s `endpoint`,
  `d2b-provider-command`'s `command`, `d2b-provider-operation`'s `operation`,
  `d2b-provider-quota`'s `quota`, `d2b-provider-zone-link`'s `zone_links` and
  `zonelink`, `d2b-contracts`' `privileges_w3`, and
  `d2b-contracts-broker`'s `broker_wire`. `d2b-contracts`' `opaque_payload`
  keeps its module arm too, the path that carries its surface, and loses the
  single-item root re-export.
- The modules whose consumers already used the crate root are private now, so
  their items stay reachable at exactly the crate root and a new item inside
  them can no longer widen the public API by accident:
  `d2b-resource-api::client`, `d2b-provider-guest-cloud-hypervisor::adoption`,
  `d2b-provider-guest-azure-virtual-machine::error`,
  `d2b-provider-toolkit::base`, `d2b-provider-guest-qemu-media::controller`,
  and `d2b-controller-toolkit::contract`. Their crate-root re-exports still
  carry every item the module exposed; the toolkit root re-export now also
  names `PlannedStep`, the item `StartupPlan::steps` returns.
