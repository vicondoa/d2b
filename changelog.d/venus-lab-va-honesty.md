### Fixed

- Corrected an overstated claim in the Venus Vulkan Video lab prototype. Enabling
  the virgl video backend makes the guest advertise H.264 through VA-API, and
  that is what Firefox's capability probe reads, but the advertisement was
  described as a proven capability without the measurement that would establish
  it. A guest VA-API decode has since been measured against the same decode on
  the host: the host engaged the hardware decoder at 94 to 98 percent while the
  guest engaged it at zero percent on every sample, running well faster than the
  hardware itself. Removing the preference therefore replaces a bypassed probe
  with an unverified advertisement rather than a demonstrated capability. What
  decodes is unchanged and unaffected: Firefox decodes through Vulkan Video,
  which is measured as hardware backed, and never through VA-API.
