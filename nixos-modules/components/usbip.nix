# USBIP YubiKey passthrough for nixling VMs. Imported by host.nix
# whenever a VM sets `nixling.vms.<name>.usbip.yubikey = true`.
#
# This file holds only the GUEST-side wiring:
#   - vhci_hcd kernel module so `usbip attach` can materialise the
#     redirected device as /dev/hidraw<N> inside the VM kernel.
#   - usbip CLI tools so the guest can issue `usbip attach`.
#
# The HOST-side bits (broker-spawned per-env usbipd/proxy runners,
# usbip-host kernel module, udev rules granting kvm-group access to
# Yubico hidraw + raw USB nodes) live outside this guest component
# because they're shared across VMs and depend on the host bridge
# being up.
#
# The hot-plug ceremony (bind on host, attach in VM, cleanup on
# exit) lives in the `nixling` CLI (modules/nixling/cli.nix).
{ pkgs, ... }:

{
  boot.kernelModules = [ "vhci_hcd" ];

  environment.systemPackages = [ pkgs.linuxPackages.usbip ];
}
