### Added

- Added the host-integration guest image as a Bazel build action. `bazel build //bazel/checks/vm:guest_image` realizes the guest's NixOS closure from declared label inputs - the flake, its lock, the d2b module sources, and the nine d2b host binaries plus the Cloud Hypervisor controller - so the image is a graph output that rebuilds when a guest module or a host binary changes, and a second build of an unchanged tree reuses it. The action configures its own nix store, substituters, and build-users setting, and preflights the substituters before building, so the image no longer depends on the developer's shell configuration. The guest's `d2bHostToolOverrides` now come from that declared label set rather than from an environment variable.
- The action's output is the bootable guest image, not a copy of the system closure's symlink tree: a kernel, an initrd, and a qcow2 root disk, built the way NixOS's own VM module builds them, alongside a manifest naming those artifacts, the host-tool package the closure was built against, and the per-check invocation shape. A toplevel symlink tree is not an image a lane can boot, and copying it into a build output only produces symlinks that resolve outside it. The disk's filesystem UUID and build clock are pinned, so the same inputs produce the same bytes and the image stays cacheable.
- Added the emulator to the pinned nix package extension in `MODULE.bazel`, so the lane's emulator comes from the same nixpkgs revision the guest image is realized from.

### Changed

- Factored the flake's Bazel host-tool wiring into one helper shared by both guest entry points, so a guest realized from the declared-input action and a guest realized from the legacy `D2B_HOST_TOOL_BUNDLE` environment handoff are built the same way. Both environment reads remain until the Bazel lane replaces the nix recipe.
- `make test-host-integration` now builds the guest's host tools under the committed `guest` profile from `.bazelrc`, so an exported Bazel profile cannot change the guest closure.

### Removed

- Retired the recipe's Attic closure upload from the new guest-image path: a network side effect would make the image uncacheable, and the build cache the image lands in replaces it. The upload stays with the nix recipe and retires with the lane, which is recorded here rather than left for the next reader to rediscover.
