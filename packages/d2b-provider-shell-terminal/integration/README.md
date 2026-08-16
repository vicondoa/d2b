# Integration fixtures

This provider's cross-boundary behavior is exercised by the hermetic Rust
integration targets in `../tests/`:

- current-request authorization and relay refusal;
- workload-user Host and Guest placement validation;
- process-conformance launch and restart adoption;
- bounded output replay, stale generation refusal, and one-shot capabilities.

The tests use typed effect-port fakes. They do not spawn a host shell, connect
to a broker, or retain terminal content outside the supervisor model.
