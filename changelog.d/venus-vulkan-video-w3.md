### Added

- Added `labs/venus-vulkan-video`, an isolated prototype that lets stock,
  unmodified upstream Firefox in a guest VM decode H.264 on the host GPU through
  Venus and virtio-gpu, replacing a forked Firefox and a separate virtio-media
  V4L2 decode path. The lab carries its own isolation contract, pin and evidence
  manifest, a reversible `/dev/kvm` grant helper, a scoped teardown that cannot
  reach unrelated VMs on the same host, and capability probes that run inside the
  GPU sidecar's bubblewrap namespace. It is self-contained, requires no host
  configuration change, and is deliberately outside the framework's option
  schema and gates.
- The lab reaches its goal: the browser is unpatched, decode runs on the host
  video engine, and the picture is correct. Getting there needed four
  interlocking fixes in the guest Mesa virgl driver and the host virglrenderer,
  all concerning the import of a decoded frame whose planes share one buffer.
  The plane index has to survive an import that hits the buffer-object cache,
  that import has to be describable to the host at all, a description covering a
  newly seen plane has to leave the guest rather than be discarded as a retype,
  and the host has to build an image for that plane in a pixel format the driver
  accepts. Both forks also gained opt-in import and blit tracing, because the
  existing debug machinery compiles out in release builds and so reported
  nothing at all.
- Added `labs/venus-vulkan-video/SOLUTION.md`, recording the full account: why a
  decoded frame is one allocation with its planes at offsets, how the browser
  consumes it, each defect and its fix, the changes still required for unrelated
  reasons, the measurements, and the several plausible fixes that proved inert.
