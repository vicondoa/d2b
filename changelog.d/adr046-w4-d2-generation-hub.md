### Added

- Added internal post-commit Zone configuration generation planning for configuration-owned metadata, cleanup status, per-item name-conflict isolation, and count-bounded prior bundle retention. Production store, watch, finalizer, audit, and status adapters are not wired yet.
- Added generation bundle contract validation that rejects caller-supplied lifecycle ownership metadata and checks Provider schema digests in the core planner; full build-time schema validation and executable generation activation remain pending.
