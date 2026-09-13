//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the Guest family boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must boot the
//! daemon plane, commit a Guest row naming one runtime Provider, and prove:
//! the kind's children are created by its runtime Provider controller, the
//! Guest never goes Ready ahead of its children, a Provider the family does
//! not own is refused at validate, and deleting the Guest retires the
//! controller-owned children in their preserved order.
