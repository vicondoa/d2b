# `d2b-provider-device` integration fixtures

The hermetic suite in `tests/` drives the Device rows over a recording
Provider port. The fixtures here need a daemon, a broker, or host hardware to
say anything the hermetic suite cannot.

`device_family.rs` checks the registration shape the plane consumes: the
`Device` type declared once, over the four realizer crates' exported Provider
identities. It needs none of those services, so it runs in the default lane.
