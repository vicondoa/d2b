# `d2b-provider-telemetry-service` integration fixtures

This directory holds the heavier cross-process and Host-plane fixtures for
this crate. They cannot run at the hermetic layer that `tests/` occupies.

No scenario file lands yet. The intended host-integration scenario is a Zone's
telemetry Service observing its provider-published ingest `Endpoint` rows
through the real manager, with the readiness term proven once the
driver-facing manager surface carries a dependency's observed status.
