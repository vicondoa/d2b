//! integration-target: host-integration
//! coverage-status: declaration-only
//!
//! Scenario contract for the Endpoint provider boundary.
//!
//! No Cargo target or repository lane compiles or invokes package-local
//! scenario files. This declaration awaits host-integration orchestration and
//! must not be cited as test evidence. The future scenario must boot the
//! daemon plane, commit a binding-owned virtiofsd Endpoint row beside its
//! producer worker Process row, and prove: the socket is realized through the
//! daemon's endpoint effect port, the Endpoint row reaches `Realized`, the
//! delete leg removes the socket before the worker row retires, and a
//! guest-runtime control Endpoint row is observed through the guest's
//! committed VMM Process row rather than through a socket the daemon creates.
