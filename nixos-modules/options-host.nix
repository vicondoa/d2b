# d2b.host.* — host-level infrastructure knobs for d2b subsystems
# that are owned at the host layer (not per-VM or per-env). Currently
# contains the USB security-key proxy configuration.
#
# Contrast with `d2b.site.*`, which holds site-customisation defaults
# inherited by all VMs (waylandUser, stateDir, yubikey, …). Options
# here represent host infrastructure capabilities that must be
# explicitly enabled; they are not inherited by VMs.
{ lib, ... }:

let
  # Stable FIDO/CTAP-class USB vendor IDs (decimal). The broker ONLY
  # opens hidraw nodes that match both the configured selector AND belong
  # to a known FIDO vendor class. This is an eval-time allowlist; runtime
  # class probing adds an additional defence-in-depth layer.
  #
  # Sources: FIDO Alliance member list + common CTAP2 authenticator vendors.
  knownFidoVendorIds = [
    4176 # 0x1050, Yubico
    2414 # 0x096e, Feitian Technologies
    11415 # 0x2c97, Ledger
    8352 # 0x20a0, Clay Logic / Nitrokey
    12675 # 0x3183, NEOWAVE
    1155 # 0x0483, STMicroelectronics
    9601 # 0x2581, Plug-up International
    6724 # 0x1a44, VASCO / OneSpan
    2652 # 0x0a5c, Broadcom / BRCM Bluetooth
    6353 # 0x18d1, Google / Titan Key
    4292 # 0x10c4, Silicon Labs
    1254 # 0x04e6, SCM Microsystems / identOS
    1267 # 0x04f3, Elan Microelectronics
    9436 # 0x24dc, JNSE / JMicron FIDO
  ];

  # Sub-type for one stable selector entry under `d2b.host.usb.securityKey.devices`.
  # A selector matches a physical FIDO/CTAP device by stable sysfs
  # attributes (vendor + product ID, optional serial). The broker resolves
  # the matching hidraw node at runtime.
  securityKeyDeviceSelectorType = lib.types.submodule {
    freeformType = null;
    options = {
      vendorId = lib.mkOption {
        type = lib.types.ints.between 1 65535;
        example = 4176;
        description = ''
          USB vendor ID of the FIDO/CTAP security key (decimal integer or
          `0x`-prefixed hex literal). Must identify a device in the
          broker's FIDO-class allowlist (Yubico 0x1050, Feitian 0x096e,
          Google Titan 0x18d1, etc.). Raw vendor IDs outside the known
          FIDO-class set are rejected at eval time.

          Use host udev/sysfs inventory or `d2b usb probe` to identify the
          vendorId of a plugged-in token.
        '';
      };

      productId = lib.mkOption {
        type = lib.types.ints.between 1 65535;
        example = 1031;
        description = ''
          USB product ID of the security key. Together with
          `vendorId`, this pins the selector to a specific device model.
        '';
      };

      serial = lib.mkOption {
        type = lib.types.nullOr (lib.types.strMatching "^[A-Za-z0-9._:/-]{1,255}$");
        default = null;
        example = "D3A4C5B6";
        description = ''
          Optional serial number string for disambiguation when multiple
          identical security keys are attached. The broker matches this
          against the sysfs `serial` attribute. Leave null to match any
          device with the given vendorId + productId.

          Raw `/dev/hidrawN` paths and USB bus IDs are NOT accepted here;
          use only the stable vendorId/productId/serial triple.
        '';
      };

      label = lib.mkOption {
        type = lib.types.strMatching "^[a-z][a-z0-9-]{0,62}$";
        example = "yubikey-primary";
        description = ''
          Human-readable stable label for this device, used in status
          output, audit records, and notification messages. Must match
          `^[a-z][a-z0-9-]{0,62}$`. Unique within
          `d2b.host.usb.securityKey.devices`.
        '';
      };
    };
  };
in
{
  options.d2b.host.usb.securityKey = {
    enable = lib.mkEnableOption ''
      Host-side USB security-key proxy.

      When enabled, the d2b host broker is authorised to open the
      configured FIDO/CTAP hidraw node(s) and relay CTAP HID traffic
      to requesting guest VMs over AF_VSOCK. Guest VMs must
      individually opt in with
      `d2b.vms.<name>.usb.securityKey.enable = true`.

      Phase 1 note: security-key proxying and YubiKey USBIP
      passthrough (`d2b.vms.<name>.usbip.yubikey`) are mutually
      exclusive per VM; a VM cannot simultaneously use both proxy
      modes for the same physical key. The eval-time assertion in
      `nixos-modules/assertions.nix` enforces this constraint.

      Disabling this option removes all udev group grants and broker
      capability flags for hidraw access; it does NOT affect the
      legacy `d2b.site.yubikey.enable` USBIP path.
    '';

    devices = lib.mkOption {
      type = lib.types.listOf securityKeyDeviceSelectorType;
      default = [ ];
      example = lib.literalExpression ''
        [
          {
            label     = "yubikey-primary";
            vendorId  = 4176;     # 0x1050, Yubico
            productId = 1031;     # 0x0407, YubiKey 5 NFC
            serial    = null;     # match any serial
          }
        ]
      '';
      description = ''
        List of FIDO/CTAP security keys the host broker is allowed to
        open for VM-side CTAP HID relay. Each entry is a stable selector
        (vendorId + productId + optional serial); raw `/dev/hidrawN`
        paths and USB bus IDs are NOT accepted.

        All configured vendor IDs must belong to the FIDO-class
        allowlist (enforced at eval time). The broker opens only the
        exact devices named here; no blanket hidraw access is granted.

        Leave empty to allow no security-key access even when
        `enable = true` (the option flag is then effectively a no-op
        at runtime, though it still validates to the allowed-empty
        state at eval time).
      '';
    };

  };
}
