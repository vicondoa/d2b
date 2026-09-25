### Fixed

- The shared provider driver no longer panics when its construction zone is
  invalid: the zone is now held as a validated `ZoneId` in the driver
  arguments instead of being parsed (and panicking) at driver construction.
- `key_ref` no longer panics on a non-canonical resource key; it returns an
  error that callers propagate instead.
- The toolkit fixture no longer panics on an unknown specified provider
  method; it reports a wire-invalid error instead.
- The fake provider ports now surface a full recorder as
  `FakePortError::RecorderFull` instead of silently dropping the record.
- Provider session warnings now carry the zone and provider identity instead
  of being message-only.
- The provider adapter no longer clones the decoded request's zone, provider,
  method, and payload on every frame; it moves them into dispatch.