//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the ShellPool boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must boot the daemon plane, commit a pool row beside its sessions, and prove the pool reaches `Ready`, refuses deletion while a live session references it, and converges once the last session retires.
