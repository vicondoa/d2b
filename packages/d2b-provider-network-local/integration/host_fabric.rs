//! integration-target: container
//!
//! Host-fabric lifecycle scenario contract.
//!
//! The executable scenario is enabled when the core effect adapter and the
//! closed bridge, persistent-TAP deletion, and ownership-projection broker
//! operations have production handlers. Until then, hermetic tests in this
//! crate prove the projection preservation, marker rejection, bridge-port
//! readback, route readiness, and sysctl ordering used by that adapter. This
//! file intentionally names no alternate host mutation path.
