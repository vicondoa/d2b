# Security-key CTAPHID proxy — guest-side component.
# Imported into a realm-owned local VM workload configuration when that
# workload requests FIDO security-key mediation.
#
# This module wires:
#   - The `uhid` kernel module so the frontend binary can open /dev/uhid.
#   - A `plugdev` group and udev rules granting group-readable access to
#     /dev/hidraw* nodes so Firefox/libfido2 can access the virtual key
#     without root.
#   - The `d2b-sk-frontend` static binary from the framework packages.
#   - A guest-side systemd service that runs the frontend and reconnects
#     to the host broker over AF_VSOCK with exponential backoff.
#
# The host-side DAG node (declared in nixos-modules/processes-json.nix)
# tracks the vsock endpoint readiness from the daemon's perspective.
# The guest-side process is supervised by the guest's systemd.
#
# The virtual HID device created by d2b-sk-frontend is visible to
# libfido2 and Firefox as a standard FIDO2/CTAPHID authenticator.
# `fido2-token -L` inside the guest should enumerate it immediately
# after the module is activated.
{ config, lib, pkgs, d2bWorkloadId, d2bInputs, ... }:

let
  cfg = config.d2b.securityKey;
  guestPackages = d2bInputs.self.packages.${pkgs.stdenv.hostPlatform.system};
  skBinary = "${guestPackages.d2b-sk-frontend-static}/bin/d2b-sk-frontend";
in
{
  options.d2b.securityKey = {
    vsockPort = lib.mkOption {
      type = lib.types.int;
      default = 14320;
      internal = true;
      readOnly = true;
      description = ''
        AF_VSOCK port the guest sk-frontend connects to on the host for
        CTAPHID relay. Must match the port the host broker listens on.
        Internal option: do not override in VM config.
      '';
    };
  };

  config = {
    # Load the UHID kernel module so /dev/uhid is available.
    boot.kernelModules = [ "uhid" ];

    # Create the plugdev group if not already declared. Workload users that run
    # browsers/libfido2 should join this same group for hidraw access.
    users.groups.plugdev = { };

    # udev rule: grant the `plugdev` group read/write access to the virtual
    # FIDO/CTAPHID HID raw device created by d2b-sk-frontend.
    #
    # The virtual device is a UHID HID parent whose kernel name encodes the
    # Yubico FIDO vendor/product pair used by d2b-sk-frontend.
    services.udev.extraRules = ''
      # d2b security-key: grant plugdev access to FIDO/CTAPHID HID raw nodes.
      # Match the virtual FIDO device created by d2b-sk-frontend.
      KERNEL=="hidraw*", SUBSYSTEM=="hidraw", SUBSYSTEMS=="hid", KERNELS=="0003:1050:0407.*", GROUP="plugdev", MODE="0660"
    '';

    # The d2b-sk-frontend service runs as the guest's login user so it has
    # access to /dev/uhid (mode 0660, owned root:input by default on most
    # distros; uhid is world-read/write on NixOS). We add the user to the
    # input group for good measure.
    #
    # The service is Restart=always to recover from vsock disconnects and
    # host broker restarts automatically.
    systemd.services.d2b-sk-frontend = {
      description = "d2b virtual FIDO/CTAPHID security-key frontend";
      wantedBy = [ "multi-user.target" ];
      after = [ "local-fs.target" "systemd-udev-settle.service" ];
      startLimitIntervalSec = 0;
      # Do not require the udev settle: the uhid module may not be loaded
      # yet at service start, but the binary will retry on failure.

      environment = {
        D2B_SK_VM_ID = d2bWorkloadId;
        D2B_SK_VSOCK_PORT = toString cfg.vsockPort;
        # /dev/uhid is always at this path on Linux.
        D2B_SK_UHID_PATH = "/dev/uhid";
      };

      serviceConfig = {
        ExecStart = skBinary;
        Restart = "always";
        # Wait 2s before the first restart attempt to avoid spinning on a
        # kernel that is still loading the uhid module.
        RestartSec = "2s";
        # Not privileged: runs as root only to open /dev/uhid (mode 0600
        # root:root on some kernels). If the guest sets uhid to 0660
        # root:input, we can drop to the login user with
        # User = config.d2b.guestControl.exec.execUser or similar.
        # Keep as root for simplicity; the uhid fd scope is limited to the
        # virtual device we own.
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        NoNewPrivileges = true;
      };
    };
  };
}
