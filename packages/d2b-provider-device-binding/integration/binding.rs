//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the DeviceBinding provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must boot the
//! daemon plane, commit a `DeviceBinding` row the Device source admitted for a
//! present named function, and prove: the row's attachment reaches the
//! consumer and the fenced readiness projection reports it under the row's own
//! fence, a daemon restart re-adopts the realized attachment instead of
//! attaching the consumer a second time, an inventory that stops backing the
//! named function is refused terminal without a second claim, a consumer that
//! still holds the attachment blocks the teardown, and the delete leg releases
//! the attachment before the device slot.