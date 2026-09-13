# `d2b-provider-telemetry-binding` integration fixtures

This directory holds the heavier cross-process and Host-plane fixtures for
this crate. They cannot run at the hermetic layer that `tests/` occupies.

No scenario file lands yet. The intended host-integration scenario is a Zone
telemetry Binding materializing its provider-declared collector Process and
ingest Endpoint through the real manager, with the Process controller
launching the collector, then retiring the child set endpoint-first.
