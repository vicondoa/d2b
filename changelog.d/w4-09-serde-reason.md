### Fixed

- Wayland-family Provider wire-parse failures no longer collapse into a bare
  invalid-resource error: a spec or resource envelope that fails to parse
  now carries the serde reason (field, line, column) through the
  `d2b-provider-wayland-policy` error surface, so operator diagnostics
  report why a row was refused instead of a generic invalid-resource code.