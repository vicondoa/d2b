### Changed

- Launching a QEMU media guest now preallocates the launch-ticket attachment
  list to its known upper bound, avoiding reallocation churn on the launch
  path.
- The qemu-media lifecycle test suite builds its device observation from a
  shared fixture instead of eight near-identical literals, so future field
  changes touch one place.