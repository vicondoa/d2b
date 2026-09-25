### Fixed

- Provider manifest supply cleanup: `d2b-provider-audio-pipewire` drops the
  unused `schemars` dependency and moves `serde_json` to dev-dependencies;
  `d2b-provider-device-gpu` drops unused `async-trait` and
  `d2b-resource-types` (manifest and Bazel deps); `d2b-provider-quota` drops
  unused `serde_json`; `d2b-provider-guest-azure-container-apps` drops unused
  `sha2`.
- `d2b-provider-transport-azure-relay` upgrades its direct `webpki-roots` pin
  from 0.26 to 1, collapsing the duplicate roots leg already pulled in by
  `tokio-tungstenite`'s rustls-tls-webpki-roots feature.