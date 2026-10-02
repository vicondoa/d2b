### Removed

- Removed the broker's bootstrap-shaped wire. The `layer1-bootstrap` Cargo
  feature, the feature it forwarded through the broker composition root, the
  `d2b-broker-layer1-bootstrap` binary target, the `d2b_broker::bootstrap`
  module and every `cfg` arm it selected are gone. The broker now has one
  wire: the opaque-ID `BrokerRequest` contract its `live_handlers` route.
  Building the broker with the old feature is no longer possible, so the
  stale-wire test harnesses that only ever ran against the bootstrap shape
  were deleted rather than left to rot.
- Removed the `unknown-operation` broker refusal. It existed only for the
  bootstrap dispatcher's USBIP live-device-routing arm, which is not a
  production arm.
- Removed the installed `privileges.json` artifact. The Nix module that
  generated it, its bundle-installation wiring, its per-artifact hash input,
  and the `privilegesPath` key in `bundle.json` are all gone, along with the
  `privileges_path` field on the bundle index the daemon and broker parse.
  Nothing read the file: the bundle index carried the path and the broker
  never opened it, so removing it changes no runtime behaviour. The broker
  operation authorization rows continue to come from the generated
  `broker_operation_authz.rs` view of the operation catalog.

### Changed

- The bundle index no longer carries `privilegesPath`. A `bundle.json` that
  still declares it is rejected, because the index denies unknown fields.