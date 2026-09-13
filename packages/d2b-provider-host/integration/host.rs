//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the Host provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must boot the
//! daemon plane, commit the bootstrap `Host/host-system` row, and prove: the
//! row validates through the declared decoder and the `Provider/system-core`
//! fence, the bounded capability/platform/metadata probe observes the local
//! host through the daemon's effect port, the published status carries that
//! observation, a probe that cannot complete still converges as a degraded
//! observation rather than failing the resource, and a second reconcile pass
//! at the same generation re-probes nothing.
