### Changed

- Driver-effect test doubles are shared, not duplicated: the guest, wayland-policy,
  host, user, provider, and notification-provider crates now publish their recording
  double from a `test_support` module behind a `test-support` feature, and both their
  own tests and the daemon's plane tests consume that one implementation instead of
  per-file copies.
- Every surviving `#[allow(dead_code)]` in the daemon and the resource API now states a
  factual reason at its site. Notes that pointed at a possible future consumer or at a
  plan unit that has since landed were replaced with the actual state: a test-only
  harness surface with no production caller, or the shared harness every integration
  test pulls in with `mod common;`.
- The dead-code scan runs from the development shell. `cargo-shear` is provisioned in
  the `d2b-dev` shell and the `check-dead-code` target joins the local dispatch class,
  so it enters that shell instead of failing on a missing tool.

### Removed

- The per-env USBIP autostart seam in `d2bd-runtime`, with its eleven unit tests and its
  row in the runtime-boundary file table, and the ADR-046 device dossiers updated to
  describe the deletion as landed rather than pending. Its symbols had no consumer
  outside the module; the USBIP provider's own tests are the parity evidence the
  dossier's removal condition asks for.
- The resource runtime's internal routing request enum and its channel-backed endpoint
  wrapper, along with the unused imports they held. The five unit tests that drove them
  were reworked onto a local endpoint stub with their assertions intact.
- The resource compiler's `compile_provider_artifact` alias; the canonical
  `compile_artifact` it forwarded to stays.
- The controller's binding-child reconciler type and its re-export, with the five tests
  and one helper whose only subject it was.

### Retained (recorded reasons)

- The neutral volume effect-port contract and its generic host wrapper. They are a
  scheduled interface, not residue: the volume-local provider's completion task assigns
  the adapter that implements them to unit `ADR046-vl-012` (task T469), and the debt
  ledger records that adapter as not yet built.
- `WatchSink` in the resource API: its file states that nothing produces frames until
  the manager-side pump lands, and its bus implementation carries the tested
  watch-delivery credit path.
- `LiveControllerSessionEvidence` in the daemon plane bridge: it disambiguates two
  effect traits implemented by one type, and its reason is stated in the file.
- The store's audit-log history read path: it has no production caller, but five
  in-crate tests use it as their audit observation mechanism, including the
  durability assertions that pin the zero-persistent-write property.
- The resource-runtime identity re-export module: ten in-crate modules and nine
  provider-side crates import their identity types through it.