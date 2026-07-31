### Added

- Added strict Credential lease and status contracts plus an exact five-method Credential service with one-way opaque identifiers and end-to-end sensitive-delivery bindings.

### Security

- Kept token, signature, lease source, and credential identity material out of outer service DTOs and diagnostic output, with explicit zeroization for plaintext delivery records.
