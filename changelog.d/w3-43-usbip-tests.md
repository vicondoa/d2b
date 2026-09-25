### Changed

- The USBIP provider test suite now covers the device-claim arbiter's ceiling,
  arbitration-mode, re-claim, and release paths, and pins the wire shape of the
  USB event source, reconcile context, public degraded reason, and claim source
  payloads, so kebab-case/camelCase wire drift fails the build instead of
  shipping silently.