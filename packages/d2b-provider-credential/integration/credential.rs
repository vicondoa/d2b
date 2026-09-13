//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the Credential provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must boot the
//! daemon plane, commit a managed-identity Credential row beside its Provider
//! row, and prove: the declared agent Process child is minted under
//! `Provider/system-minijail` and the Credential reaches `Ready` only once
//! the child serves, a delete revokes the lease through the authenticated
//! Provider session before the owned child is marked deleting, and a
//! Credential whose session generation is missing or stale fails closed with
//! no child deletion.
