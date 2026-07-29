# Local development shell for the d2b-wlattach prototype.
#
# This crate is deliberately outside CI (see README), so this shell is the
# supported way to build and run it. It is not referenced by the repo flake.
{ pkgs ? import <nixpkgs> { } }:
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    pkg-config
  ];
  buildInputs = with pkgs; [
    # smithay links libxkbcommon.
    libxkbcommon
    # Wayland client/server plumbing.
    wayland
    # Probes: DRM dumb-buffer allocation and dmabuf export.
    libgbm
    libdrm
  ];
  packages = with pkgs; [
    # Inspection and validation tooling used by the phase gates.
    wayland-utils   # wayland-info
    vulkan-tools    # vkcube: confirmed dmabuf client on this host
    gtk4            # gtk4-demo
    foot            # SHM control client
  ];
  shellHook = ''
    export LIBRARY_PATH="${pkgs.libxkbcommon}/lib:''${LIBRARY_PATH:-}"
    echo "d2b-wlattach dev shell — cargo test --locked"
  '';
}
