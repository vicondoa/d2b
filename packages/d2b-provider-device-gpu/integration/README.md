# `device-gpu` integration fixtures

`provider_lifecycle.rs` declares the `host-integration` target. The scenarios
run through `make test-host-integration` once the Core effect adapter and
Host/Guest harness are connected; they are not standalone scripts.

| Fixture | Required end-to-end assertion |
| --- | --- |
| `gpu_worker_start/` | worker receives only Core-issued LaunchTicket device grants and reaches Ready |
| `render_node_shared/` | two explicit shared render-node claims are admitted without widening authority |
| `video_dependency/` | the separate video worker starts only after GPU readiness |

The ordinary integration lane uses approved fake device inventory. Real GPU
hardware belongs only to the repository hardware lane. The hermetic Cargo
tests in `tests/` cover the corresponding settings, authority, sequencing,
process-name, and wire-contract invariants without opening host devices.
