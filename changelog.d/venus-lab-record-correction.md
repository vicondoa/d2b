### Changed

- The Venus Vulkan Video lab prototype no longer asserts a graphics capability
  its guest does not have. It previously set
  `gfx.blacklist.hardwarevideodecoding` so Firefox would skip a VA-API probe the
  guest could not pass; the guest now passes that probe on the merits and the
  preference is removed. Decode remains Vulkan Video through Venus, with VA-API
  answering only the capability question. Firefox is still unmodified.
- The lab's virglrenderer fork can initialise its video backend and accept the
  host's non-Mesa VA driver, each behind its own explicit opt-in that is off by
  default. Without them the fork behaves exactly as upstream does.

### Fixed

- Corrected lab findings documents that described the green-frame presentation
  defect in the present tense after it had been root-caused and fixed, and that
  described the GPU-copy path as the intended target when the working
  configuration selects zero copy.
- Corrected lab flake comments that stated the opposite of the preferences
  declared beneath them, including one claiming direct export was enabled while
  the preference set it to false.
