### Fixed

- Invalid generated Zone resource bundles now fail closed during compilation,
  while telemetry and audit bounds remain shared by the typed resource surface.
- Telemetry redaction policy checks now cover multiline field forms across the
  relevant Rust source without allowing unbounded source exemptions.
