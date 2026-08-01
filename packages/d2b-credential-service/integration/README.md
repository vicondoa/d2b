# `d2b-credential-service` integration fixtures

No heavy fixture is implemented in this crate. Cross-process bus routing and
end-to-end encrypted delivery require the future authenticated production bus
registration. A future Rust scenario file in this directory must declare
exactly one `//! integration-target: container` or
`//! integration-target: host-integration` line in its first 20 lines.
