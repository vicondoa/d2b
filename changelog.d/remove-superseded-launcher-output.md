### Removed

- Removed the unused v1 realm workload launcher artifact and its Nix and
  fixture registration; launcher clients now use the provider-neutral v2
  metadata.
- Moved retained host-tool package option declarations out of the gateway
  compatibility tombstone without changing host-tool resolution.
