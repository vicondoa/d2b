---
type: added
area: providers
---

### Added

- **A committed USB Service or security-key Binding row now reaches a bounded
  helper leg on the production path.** Each family declares a
  `...HelperLegSource` facet the composition root supplies over the `Device`
  source, together with a `...ClaimPortSource` for the privileged effects. The
  `Device` source mints the admitted relationship and the leg as one pair, so
  neither half can be combined with a claim from another relationship. A plane
  with no graph authority behind it holds a named refusal that admits nothing
  and grants nothing, and it reports the refusal rather than a silent success.
- **The relay helper is derived, not named.** USBIP names its own
  `Process/usbip-relay` worker and the security-key family derives its relay row
  from the admitted relationship's device identity, so neither a committed row
  nor an API caller chooses which `Process` realizes the claim.

### Fixed

- **A semantic USB Service or security-key Binding no longer decides its own
  device.** The claim and the leg are verified before the port is reached, so a
  cross-Zone, a stale store, or a foreign-device claim leaves no relay, no open
  hidraw node, and no firewall projection behind it. A device that reappears
  under a different physical authority is reported as replaced rather than
  carried forward under a previous owner's reservation.
