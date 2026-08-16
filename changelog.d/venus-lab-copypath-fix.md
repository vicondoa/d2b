### Fixed

- Repaired the Venus Vulkan Video lab prototype's GPU-copy presentation
  fallback, which rendered the video area green because the chroma plane was
  never copied. A blit does not go through a sampler view, so it did not reach
  the per-plane images that the preferred zero-copy route resolves through its
  sampler views, and it could not be made to: the hardware copy call derives
  formats from the texture objects and requires a shared texel size class, while
  a texture bound from an imported image reports no internal format at all.
  Sampling has no such requirement, so the per-plane blit now takes the shader
  path, joining the colour-space-conversion case already excluded from the copy
  fast path for a related reason. Blit failures fall to zero and the picture is
  correct.
- The zero-copy route remains the preferred and selected one, and is unaffected:
  it issues no blits, so the repaired path is never reached from it. Re-measured
  after the change with no failures on any surface and a correct picture.

### Added

- An opt-in host trace for per-plane blit resolution in the lab's renderer fork,
  reporting a separate count for each condition that can reject a plane lookup
  and the plane texture's actual internal format and size. A lookup governed by
  several conditions returns a single negative result that names none of them,
  which had previously been read as the resolution failing when it was
  succeeding.
