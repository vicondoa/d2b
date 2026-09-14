//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the AudioService boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must boot the daemon plane, commit an audio service row beside the binding that references it, and prove the service admits, reconciles to `Ready` through the Provider effect, and refuses deletion while a live binding still references it.
