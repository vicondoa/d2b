//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the User provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must boot the
//! daemon plane, commit a `User` row for an identity the host actually
//! resolves, and prove: NSS discovery runs through the daemon's effect port
//! rather than in the driver, the published status carries the resolved phase
//! and only the opaque identity digest, a User the host does not resolve stays
//! `Pending` rather than failing, an identity whose declared group
//! memberships do not verify is reported as drifted, and a second reconcile
//! pass at the same generation re-discovers nothing.
