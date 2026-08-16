### Added

- Added strict Credential lease and status contracts plus an exact five-method Credential service implementation with one-way opaque identifiers and authorization-owned delivery bindings. The service remains unregistered and has no production bus, Provider selection, or encrypted forwarding path.

### Security

- Service contracts keep token, signature, lease source, and Credential identity material out of outer DTOs and diagnostics, with explicit zeroization for plaintext delivery records; these guarantees are currently exercised only in hermetic service tests.
