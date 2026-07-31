### Fixed

- Named the exact blocker behind the Venus Vulkan Video lab prototype's guest
  VA-API decode failure, rather than leaving it as upstream work of unknown
  size. The host driver rejects the decode at its hardware entry point with an
  invalid-value error because virglrenderer supplies no per-slice decode
  parameters, and it supplies none because the virgl video wire format carries
  only a slice count and no per-slice size, offset, type, or first-macroblock
  field. Drivers that re-parse the bitstream themselves do not need those
  fields, which is why the format never carried them and why the restriction to
  those drivers is accurate rather than merely cautious. Lifting it requires
  extending the wire format across guest and host for every codec, which is a
  protocol change and is not needed by this prototype, whose decode path is
  Vulkan Video.
