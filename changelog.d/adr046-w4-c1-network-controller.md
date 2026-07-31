### Added

- Added the local Network provider controller, generic net-VM system module, typed config Volume delivery, ownership-scoped firewall reconciliation, and generation-fenced bridge and persistent-TAP cleanup.

### Security

- Network cleanup now rejects stale configuration generations and foreign ownership markers before host mutation, while preserving sibling Network, device-owned, and foreign firewall state.
