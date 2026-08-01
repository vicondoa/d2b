# `d2b-process` hermetic tests

Unit coverage in `src/backend.rs` pins value-free diagnostics and the opaque
request/observation/handle boundary. Cross-process descriptor and process
identity scenarios live under `integration/` because they require Linux process
and pidfd behavior rather than a scripted port.
