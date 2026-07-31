### Added

- Added the USBIP Device Provider's ownership-scoped firewall and relay lifecycle, including confirmed projection removal before authority release.
- Added Host-global external physical-NIC authority admission that rejects cross-Zone bridge multiplexing before any host effect.

### Security

- Kept USBIP attachment scoped to its declared Zone and prevented work and personal Zones from sharing one physical NIC's layer-2 broadcast domain.
