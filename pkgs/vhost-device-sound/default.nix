# vhost-device-sound v0.3.0 overlay.
#
# Why: nixpkgs ships v0.2.0, which has a known PipeWire-backend format
# negotiation bug that manifests as static / chopping when Firefox or
# any non-trivial PipeWire client plays through the guest sink. v0.3.0
# includes the fix:
#
#   [#884] vhost-device-sound/pipewire: fix wrong format
#
# Upstream: https://github.com/rust-vmm/vhost-device
# Tag:      vhost-device-sound-v0.3.0
#
# The package's NixOS expression itself doesn't change (CLI is
# compatible — same --socket / --backend flags). We override .version,
# .src, .cargoBuildFlags (disable the new gst-backend that needs
# additional gstreamer libs), and .cargoHash on top of nixpkgs.
{ pkgs }:

pkgs.vhost-device-sound.overrideAttrs (old: rec {
  version = "0.3.0";
  src = pkgs.fetchFromGitHub {
    owner = "rust-vmm";
    repo = "vhost-device";
    tag = "vhost-device-sound-v${version}";
    hash = "sha256-sp8henKhaOxZKpKIfz6Z9Xe1sEkSMQpwn1JJogU6bKc=";
  };
  cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
    inherit src;
    name = "vhost-device-sound-${version}-vendor";
    hash = "sha256-2rPeef/1DXSUZx8Gve9An+dgdyg2PiGbNI1aYTGxEWg=";
  };
  # v0.3.0 enables a new gst-backend feature by default which pulls in
  # gstreamer dependencies (glib-2.0.pc etc). We don't need GStreamer
  # — `--backend pipewire` is what we use. Disable default features
  # and enable just alsa-backend + pw-backend.
  cargoBuildFlags = [
    "--package" "vhost-device-sound"
    "--no-default-features"
    "--features" "alsa-backend,pw-backend"
  ];
  cargoTestFlags = [
    "--package" "vhost-device-sound"
    "--no-default-features"
    "--features" "alsa-backend,pw-backend"
  ];

  # Phase 4 multi-arch: the vhost-device-sound binary itself could in
  # principle build on aarch64-linux (it's pure Rust), but in nixling
  # it is only ever wired into a cloud-hypervisor VM via the
  # `--generic-vhost-user` flag added in CH v52. The spectrum-ch
  # build that ships that flag is x86_64-only (see
  # pkgs/spectrum-ch/), and the audio component on the host side
  # (components/audio/host.nix) refuses to evaluate on non-x86_64
  # via the host.nix platform gate. Pin platforms to x86_64-linux so
  # the dependency graph is internally consistent and downstream
  # `nix flake check --all-systems` doesn't pretend the package is
  # portable.
  meta = (old.meta or { }) // {
    platforms = [ "x86_64-linux" ];
  };
})
