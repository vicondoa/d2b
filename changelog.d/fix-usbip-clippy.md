### Fixed

- `d2b-broker`'s `W6UsbipOperation` variants no longer repeat the enum's
  `Usbip` prefix, clearing `clippy::enum_variant_names` once the module is
  no longer part of the crate's exported API; each variant keeps its exact
  kebab-case wire label (`usbip-bind`, `usbip-unbind`,
  `usbip-proxy-reconcile`) through an explicit `#[serde(rename = "...")]`,
  so the audit discriminants are byte-identical.
