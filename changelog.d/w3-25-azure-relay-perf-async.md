### Fixed

- Azure Relay sends are now cancellation-safe: a send cancelled between
  credit reservation and socket write returns the reserved credits instead
  of permanently starving the connection of up to 64 KiB of send credit.
- Azure Relay per-frame I/O no longer rebuilds the generation-fence key
  (three heap allocations) on every send and receive; the key is computed
  once per connection.
- Gateway credential policy files are read into a buffer pre-sized from the
  file metadata, avoiding reallocations during the read.