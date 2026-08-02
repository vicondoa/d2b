### Added

- Added Device resource contracts and provider implementations for TPM,
  USBIP, security-key, and GPU/video hardware, including bounded lifecycle
  controllers, opaque host effects, and hermetic validation.
- Added Zone Device authoring validation and provider layout checks.

### Changed

- Device configuration now emits canonical, provider-bound resource
  specifications without exposing host paths or runtime management state.

