# wl-cross-domain-proxy: guest-side virtio-gpu cross-domain Wayland proxy.
#
# This package provides the plain Wayland-over-virtio-gpu transport inside
# d2b graphics VMs. It intentionally does NOT perform filtering, global
# hiding, app-id rewriting, or Xwayland proxying — those responsibilities
# belong to the host-side d2b-wayland-proxy binary. The split is
# deliberate: wl-cross-domain-proxy only bridges the kernel virtio-gpu
# cross-domain transport to a guest Wayland socket; every security-relevant
# decision is made on the host side before frames reach the real compositor.
#
# Source: https://codeberg.org/drakulix/wl-cross-domain-proxy
# Rev:    c6ce1ca89fb4d6f4f18d3aaf88324d40d4589177 (main, 2025-era snapshot)
# License: MIT
#
# Build inputs:
#   drm-sys  → pkg-config + libdrm (ioctl bindings for DRM render nodes)
#   wayland-sys → pkg-config + wayland (libwayland-server for the server
#                 socket; wayland-scanner for protocol codegen at build time)
#
# UPDATE PROCEDURE
# 1. Get the new commit SHA:
#      git ls-remote https://codeberg.org/drakulix/wl-cross-domain-proxy.git HEAD
# 2. Prefetch the source hash:
#      nix-prefetch-url --unpack https://codeberg.org/drakulix/wl-cross-domain-proxy/archive/<SHA>.tar.gz
#      nix hash convert --hash-algo sha256 --to sri <BASE32-HASH>
# 3. Prefetch the cargo vendor hash (fakeHash triggers the mismatch message):
#      nix-build pkgs/wl-cross-domain-proxy --no-out-link  # with fakeHash in cargoDeps
# 4. Substitute the printed hash into cargoDeps below.
# 5. Run: nix build .#packages.x86_64-linux.wlCrossDomainProxy

{ pkgs }:

pkgs.rustPlatform.buildRustPackage rec {
  pname = "wl-cross-domain-proxy";
  version = "0.1.0";

  src = pkgs.fetchzip {
    url = "https://codeberg.org/drakulix/wl-cross-domain-proxy/archive/c6ce1ca89fb4d6f4f18d3aaf88324d40d4589177.tar.gz";
    hash = "sha256-ydyT4DFzWzhzOZR591UOgLjVQt/v6hRSNjzM3QtohlU=";
  };

  cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
    inherit src;
    name = "${pname}-${version}-vendor";
    hash = "sha256-k3dmxIuCQoOrn/VwauTdzuRw/XKQB6LPLgO5ql0rE7E=";
  };

  # drm-sys links against libdrm via pkg-config; wayland-sys links
  # libwayland-server and invokes wayland-scanner for protocol codegen.
  nativeBuildInputs = [ pkgs.pkg-config ];
  buildInputs = [
    pkgs.libdrm
    pkgs.wayland
  ];

  # Guest-only: requires a DRI render node exposed by virtio-gpu.
  # The virtio-gpu driver is not present on the host (or any non-VM
  # context). The binary can also only be usefully tested inside a VM
  # where the virtio-gpu cross-domain context is active.
  meta = {
    description = "Plain Wayland proxy over virtio-gpu cross-domain contexts";
    longDescription = ''
      wl-cross-domain-proxy is a minimal guest-side Wayland proxy that
      bridges Wayland clients inside a VM to the host compositor via the
      virtio-gpu cross-domain kernel interface. It does not filter globals,
      rewrite app IDs, or proxy Xwayland — those are host-side concerns.
    '';
    homepage = "https://codeberg.org/drakulix/wl-cross-domain-proxy";
    license = pkgs.lib.licenses.mit;
    mainProgram = "wl-cross-domain-proxy";
    platforms = [ "x86_64-linux" "aarch64-linux" ];
  };
}
