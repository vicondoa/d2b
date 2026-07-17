# USBIP security-key passthrough for realm-owned local VM workloads.
#
# This file holds only the GUEST-side wiring:
#   - vhci_hcd kernel module so `usbip attach` can materialise the
#     redirected device as /dev/hidraw<N> inside the VM kernel.
#   - usbip CLI tools so guestd can perform authenticated guest-side import
#     cleanup/attach over guest-control.
#
# Host access remains behind the host-mediated device provider and a
# local-root allocator lease. The guest receives only the mediated USBIP
# connection; physical bus IDs never become runtime path components.
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
