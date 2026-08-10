# device-security-key integration fixtures

The package-local integration target is `provider_lifecycle.rs`; it declares
the `container` lane and runs through Cargo's integration-test harness. It
uses a fake Core effect port to exercise the public Provider boundary without
opening a host device, resolving a transport address, or owning credentials.

The scenario covers:

- catalog-derived semantic Service/Binding descriptor and projection branch;
- empty semantic backing allowlist denial;
- physical backing admission before the relay open;
- one active lease and terminal release;
- same-Zone Guest frontend placement;
- opaque CID round trips and bounded session records.

The fixture directories document the future Host/Guest orchestration surfaces:

- `lease_acquire_cancel/` for acquire, cancel, and re-acquire;
- `session_ring_capacity/` for bounded ring behavior under relay load;
- `guest_frontend_connect/` for Guest frontend authentication over the
  Provider transport.

Those scenarios require the existing Core adapter and Host/Guest lane. They
must not be replaced by a Provider-owned broker, filesystem, or credential
implementation.
