//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the Volume provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must boot the
//! daemon plane, commit a Volume row whose attachments name a Guest, and
//! prove: the volume-local layout effect runs exactly once through the
//! daemon's volume effect port, the deterministic `VolumeBinding` child per
//! attachment is committed before its actor exists, a restarted daemon
//! adopts the existing layout without re-running the effect, and delete
//! removes the Volume's own layout state while the manager retires the
//! binding children.
