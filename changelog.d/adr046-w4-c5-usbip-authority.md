### Added

- Added internal USBIP effect and authority contracts for ownership-scoped firewall projections, relay cleanup, and confirmed projection removal before authority release. The full USBIP Device Provider and controller remain future production integration work.
- Added Host-global external physical-NIC authority admission policy that refuses cross-Zone bridge multiplexing before a host effect; live host behavior is not yet covered by an executable integration scenario.

### Security

- The new admission contracts scope USBIP authority to its declared Zone and refuse sharing one bridged physical NIC across Zones; they do not claim a newly production-wired USBIP lifecycle.
