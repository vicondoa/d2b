### Fixed

- Driver args across the interaction, volume-binding, guest, and shared-provider families now carry the zone as a validated `ZoneId` instead of a bare string re-parsed with `expect` at construction; the daemon boundary passes the parsed value once.
- The wayland-policy `key_ref` helper now returns a typed `Result` (SpecInvalid refusal) instead of panicking on a non-canonical manager key; the malformed-zone constructor refusal arm is gone because the zone arrives pre-validated.
- The volume-binding driver derives its socket-identity bounded token once at construction instead of re-parsing the zone with `expect` on every pass.
- Removed the never-read `zone` field from `VolumeDriverArgs`; construction sites and fixtures no longer carry it.