### Added

- The trusted site-runtime contract `site.json` and its reader: the Process
  controller now launches the GPU worker with the Wayland socket the site
  itself provisions (`d2b.site.waylandUser` + `d2b.site.waylandDisplay`),
  instead of refusing with `device-worker-wayland-sock-unbound` for lack of a
  trusted source. The artifact is declared by the bundle index (`sitePath`)
  and emitted by `nixos-modules/site-json.nix` from the same option pair the
  session wiring uses, so the bundle cannot name a runtime directory the site
  does not create. A site without a Wayland session emits `waylandSocket:
  null` and the GPU launch still refuses by name.

### Changed

- `d2b_core::site::SiteJson` (new DTO) validates the socket fail-closed at
  bundle load (exactly `/run/user/<uid>/<display>`, no parent components);
  `BundleResolver` loads it for zone-native bundles through the optional
  `sitePath` index field and, for legacy bundles, when `site.json` ships
  beside the other artifacts. Missing artifact keeps reading as "no site
  facts" - no bundle version bump and no reader change for existing bundles.
- `device_worker_wayland_sock` in the Process driver reads the projected
  value from the loaded bundle; the GPU argv composition renders it as
  `--wayland-sock`.
