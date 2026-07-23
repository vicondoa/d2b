# USBIP YubiKey passthrough for d2b VMs. Imported by host.nix
# whenever a VM sets `d2b.vms.<name>.usbip.yubikey = true`.
#
# This file holds only the GUEST-side wiring:
#   - vhci_hcd kernel module so `usbip attach` can materialise the
#     redirected device as /dev/hidraw<N> inside the VM kernel.
#   - usbip CLI tools so guestd can perform authenticated guest-side import
#     cleanup/attach over guest-control.
#
# The HOST-side bits (broker-spawned per-env usbipd/proxy runners,
# usbip-host kernel module, udev rules granting kvm-group access to
# Yubico hidraw + raw USB nodes) live outside this guest component
# because they're shared across VMs and depend on the host bridge
# being up.
#
# The hot-plug ceremony is daemon-owned: d2bd drives broker host
# bind/unbind and asks guestd to reconcile guest-side USBIP imports over
# authenticated guest-control. The CLI never SSHes into the guest for USBIP.
{ pkgs, ... }:

{
  boot.kernelModules = [ "vhci_hcd" ];

  environment.systemPackages = [ pkgs.linuxPackages.usbip ];

  d2b.guestControl.usbipPath = "${pkgs.linuxPackages.usbip}/bin/usbip";
}
