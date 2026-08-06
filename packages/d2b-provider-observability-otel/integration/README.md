# `d2b-provider-observability-otel` integration scenarios

These scenarios exercise the Provider boundary rather than a private
replacement path. They belong in the existing integration lanes because they
need a real Process Provider, ComponentSession transport, or NixOS journal.

| Scenario | Target | Boundary covered |
| --- | --- | --- |
| `agent_session.rs` | `container` | admitted session lifecycle and bounded diagnostic audit events |
| `ingress_pipeline.rs` | `container` | emitter, OTLP Unix, OTLP vsock, and imported-stream admission |
| `journald.rs` | `host-integration` | cgroup-scoped journald receiver and redaction configuration |

The default crate tests remain deterministic and hermetic. Each Rust scenario
declares one orchestration target in its first twenty lines and records the
production surfaces required before the lane can execute it. No scenario adds
a root service, owns host credentials, or bypasses the Provider or Process
boundaries.
