### Added

- Added separate USBIP Service and Binding lifecycle supervision so Guest attachment cleanup completes before owned device unbind and Host-global authority release.
- Added typed Core bundle projections and daemon broker composition for Host-global USBIP claims, private Binding runners, restart identity checks, and scoped cleanup.
