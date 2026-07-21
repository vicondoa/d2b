{ flakeRoot, lib, ... }:

let
  evalKinds = kinds:
    lib.evalModules {
      modules = [
        (flakeRoot + "/nixos-modules/options-realms.nix")
        (flakeRoot + "/nixos-modules/index.nix")
        (flakeRoot + "/nixos-modules/realm-device-rows.nix")
        ({ lib, ... }: {
          options.assertions = lib.mkOption {
            type = lib.types.listOf lib.types.attrs;
            default = [ ];
          };
          config.d2b.realms.work = {
            path = "work.local-root";
            providers.runtime = {
              type = "runtime";
              implementationId = "cloud-hypervisor";
            };
            providers.devices = {
              type = "device";
              implementationId = "host-mediated";
            };
            workloads.corp = {
              providerRefs = {
                runtime = "runtime";
                device = "devices";
              };
              securityKey.enable = kinds.fido or false;
              usbip.enable = kinds.usbip or false;
            };
          };
        })
      ];
    };

  fido = evalKinds { fido = true; };
  usbip = evalKinds { usbip = true; };
  conflict = evalKinds { fido = true; usbip = true; };
  fidoRow = builtins.head fido.config.d2b._index.devices.list;
  usbipRow = builtins.head usbip.config.d2b._index.devices.list;
  failedMessages = map (assertion: assertion.message)
    (lib.filter (assertion: !assertion.assertion) conflict.config.assertions);
in
{
  "usb-security-key/fido-is-provider-mediated" = {
    expr = fidoRow.mediation;
    expected = {
      authority = "host-mediated";
      attachment = "fd-only";
      broker = "realm-local";
    };
  };

  "usb-security-key/usbip-is-provider-mediated" = {
    expr = usbipRow.mediation;
    expected = {
      authority = "host-mediated";
      attachment = "fd-only";
      broker = "realm-local";
    };
  };

  "usb-security-key/modes-share-exclusive-global-lease" = {
    expr = {
      fido = {
        inherit (fidoRow) allocatorLeaseId allocatorShare;
      };
      usbip = {
        inherit (usbipRow) allocatorLeaseId allocatorShare;
      };
    };
    expected = {
      fido = {
        allocatorLeaseId = "lease-device-security-key-global";
        allocatorShare = "exclusive";
      };
      usbip = {
        allocatorLeaseId = "lease-device-security-key-global";
        allocatorShare = "exclusive";
      };
    };
  };

  "usb-security-key/mutual-exclusion-fails-closed" = {
    expr = lib.any
      (message: lib.hasInfix "USBIP and FIDO" message)
      failedMessages;
    expected = true;
  };

  "usb-security-key/selectors-are-canonical-not-physical" = {
    expr = lib.all
      (row:
        !(row ? selectorId)
        && !(row ? endpointPath)
        && (row.endpointId == null
          || (lib.hasPrefix "device-endpoint-" row.endpointId
            && !(lib.hasInfix "/" row.endpointId)
            && !(lib.hasInfix ":" row.endpointId))))
      [ fidoRow usbipRow ];
    expected = true;
  };
}
