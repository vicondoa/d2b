### Removed

- Removed the unused generic credential service crate; credential protocol
  ownership remains with the neutral contract and provider-local surfaces.

### Changed

- Moved ACA-specific effect contracts into the ACA runtime provider and removed
  stale cloud-provider contract and realm compatibility dependencies while
  preserving gateway composition and provider behavior.
