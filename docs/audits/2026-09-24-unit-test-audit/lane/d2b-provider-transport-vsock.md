# d2b-provider-transport-vsock - unit-test audit
tests: 1 · src files: 11
net: -0 tests, -0 lines

## Findings (biggest net first)
Nothing to cut. Ship.

Checked all 11 src files; the single unit test pins the `None` session-generation-fence branch of `OpenTransportRequest::validate()` (service.rs:200-208), which integration coverage never reaches (tests/service.rs and tests/open_close.rs always build requests via `with_session_generation(...)`, pinning only the `Some(0)` and `Some(1)` branches).

## Keep
- `open_request_requires_a_session_generation_fence` - pins that an `OpenTransportRequest` built via `new()` (fence `None`) fails `validate()` with `Err(ServiceError::InvalidSessionGeneration)`; only the `None` branch of `validate()` is exercised by any test in this crate (the `Some(0)` branch is pinned by `tests/service.rs::open_rejects_a_mismatched_core_generation`).
