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
//! fence, the crate's own bounded capability/platform/metadata probe observes
//! the local host (with the minijail platform gate from the daemon-supplied
//! facet), the published status carries that observation, a probe that
//! cannot complete still converges as a degraded observation rather than
//! failing the resource, and a second reconcile pass at the same generation
//! re-probes nothing.
