#![deny(unsafe_code)]

// W4-fu clean-break: the non-bootstrap runtime path is now
// supported and wires against the real opaque-ID
// `nixling_ipc::broker_wire::BrokerRequest` contract via the
// `live_handlers` module. The bootstrap path remains available
// behind the `layer1-bootstrap` feature for the legacy
// probe-hello / probe-stub / probe-export-audit test harnesses
// that the W2/W3 shell gates rely on; new code should target
// the real wire.
//
// `tests/broker-default-features-build.sh` was updated to
// reflect this clean break (default features now empty); the
// no-default-features gate at `tests/broker-no-default-features.sh`
// asserts the production binary compiles clean.

pub mod audit;
pub mod fd_passing;
// W4-fu live broker request handlers (pidfd_open + clone3-based
// spawn + reconcile-executor calls). Pure-shaped: take their
// inputs directly so the dispatch layer is the only mixer of
// wire decoding + bundle resolution + live execution.
pub mod live_handlers;
pub mod ops;
pub mod protocol;
pub mod runtime;
pub mod sys;

#[cfg(feature = "layer1-bootstrap")]
pub mod bootstrap;
