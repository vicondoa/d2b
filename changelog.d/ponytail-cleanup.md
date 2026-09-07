### Removed

- Removed the no-op `d2b-provider-device-gpu` `provider_lifecycle` integration test target. It declared scenarios for a Core adapter and Host/Guest harness that do not exist yet and contained no tests; the hermetic tests in `packages/d2b-provider-device-gpu/tests/` already cover the corresponding behavior.

### Changed

- Cleaned up dead test code: the uncalled `wait_timeout` process builder and the unused `set_mode` helper in the `d2bd` test fixtures, the never-read verb-set parameter threaded through the `d2b-bus` session admission test helpers, and the test-only magic role string in the `d2b doctor` pre-namespace posture probe, which now evaluates only real runner roles.
