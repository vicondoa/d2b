//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the EndpointBinding provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must boot the
//! daemon plane, commit an `EndpointBinding` row whose named `Endpoint` row is
//! live and whose owner is the row it delivers, and prove: validate refuses a
//! wildcard, an alternation, and a cross-Zone source selector before the row
//! is admitted; reconcile verifies the exact endpoint's effective access and
//! delivers nothing while the containing directory is listable or the inode
//! was replaced; a restart re-adopts the exact endpoint through a fresh
//! verification without re-delivering it; a consumer still attached blocks the
//! teardown; and a detached consumer's delivery is released.
