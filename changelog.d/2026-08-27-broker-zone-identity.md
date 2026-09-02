### Changed

- The broker no longer loads retired realm controller or identity metadata;
  the remaining argument placeholders are limited to the clean-break deletion
  boundary.

### Security

- Broker-local resource bindings now fence Zone and resource identity with
  immutable UIDs, generations, revisions, and provider references. Audit
  correlation uses digest-only keys, and credential or host-path fields are
  rejected.
