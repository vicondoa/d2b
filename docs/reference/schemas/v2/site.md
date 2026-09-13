# `site.json` schema

This generated schema documents the private site-runtime contract. It is the
trusted source for host session facts the daemon must not guess: today the
host Wayland socket the GPU worker renders into.

`waylandSocket` is the absolute `/run/user/<uid>/<display>` path resolved by
`nixos-modules/site-json.nix` from `d2b.site.waylandUser` and
`d2b.site.waylandDisplay` at bundle build time, so the bundle and the runtime
directory the site provisions cannot disagree. A site without a Wayland
session emits `null`; a bundle that does not ship the artifact at all leaves
its consumers unbound, and the GPU worker launch refuses by name
(`device-worker-wayland-sock-unbound`) instead of naming an invented path.

Regenerate the JSON schema from the Rust DTOs:

```bash
bazel run //packages/xtask:xtask -- gen-schemas
```
