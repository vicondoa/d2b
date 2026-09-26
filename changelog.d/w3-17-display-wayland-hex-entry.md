### Changed

- The wayland display provider's classified policy catalog is now populated
  through a plain function instead of a local macro; the interface table
  itself is unchanged.
- Durable display-process name suffixes are hex-encoded into the preallocated
  buffer without one formatting allocation per byte, so naming a display
  session no longer allocates 20 throwaway strings on the cold durable-naming
  path.