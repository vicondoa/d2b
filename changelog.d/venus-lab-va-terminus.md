### Fixed

- Located where the Venus Vulkan Video lab prototype's guest VA-API decode
  terminates, closing a question the previous entry left open. Counting the
  virgl decode path the way the Venus path was already counted shows the guest
  sends decode commands and every buffer-creation and render call on the host
  succeeds, after which the call that commits the decode is rejected by the
  host driver for every frame with a decoding error. That accounts for the idle
  hardware decoder, the impossibly fast throughput, and the absence of any error
  visible to the guest.
- Reclassified virglrenderer's Mesa-only video driver restriction from
  conservative to load bearing. Exporting a decoded surface uses a standard
  interface the host driver implements, which is why the restriction looked
  removable, but consuming the renderer's picture parameters and slice data is
  the part that fails. The lab's override is retained as an investigation tool
  with a warning naming the failing call and status, not as a capability.
