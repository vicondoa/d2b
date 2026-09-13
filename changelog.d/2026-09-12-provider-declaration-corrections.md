### Fixed

- Provider declarations now spell their ACL entries the way the closed Volume
  contract reads them: the TPM state Volume declares `rwx`/`rx`/`rw` instead of
  the POSIX symbolic `r-x`/`rw-`, which the grant decoder rejects - the durable
  Volume spec failed to decode (`volume-spec-invalid`) and the whole TPM device
  path was stranded. The storage-path projection follows with the same
  spellings.
- Ten Endpoint purposes across eight providers now use the closed tokens the
  Endpoint contract admits - `swtpm-tpm-socket`, `swtpm-control-socket`,
  `clipboard-wayland-bridge`, `security-key-ctaphid`, `usb-guest-proxy`,
  `display-wayland-cross-domain`, `notification-desktop-sink`, `qmp-control`,
  `audio-pipewire-host-worker`, `audio-pipewire-guest-agent` - instead of the
  dotted spellings that failed endpoint admission. The provider Nix tests pin
  each declaration.

### Changed

- The zone schema enforces the endpoint-purpose token: the generated v3 Endpoint
  schema constrains `purpose` to the bounded token pattern
  (`^[a-z][a-z0-9-]{0,62}$`) instead of a free bounded string, so a dotted
  purpose fails schema validation where it is written rather than at a daemon
  decode.
