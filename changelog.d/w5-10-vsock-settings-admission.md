### Fixed

- `VsockTransportSettings` now deserializes through a private wire mirror with `TryFrom` validation, so untrusted transport-settings JSON is rejected at the boundary instead of landing unvalidated for a later `validate()` call; the fields are private with accessors, and the wire field names and JSON schema are unchanged.