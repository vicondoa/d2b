### Fixed

- Removed the public `decode_document` forwarder from d2b-provider-config-nixos; the sole caller in d2bd now uses `ConfigSyncResponse::document()` directly, leaving one API path for validating a synced guest config document.