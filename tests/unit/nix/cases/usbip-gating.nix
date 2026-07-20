{ flakeRoot, lib, ... }:

let
  evalUsbip = enable:
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
          config.d2b.realms.dev = {
            path = "dev.local-root";
            providers.runtime = {
              type = "runtime";
              implementationId = "cloud-hypervisor";
            };
            providers.devices = {
              type = "device";
              implementationId = "host-mediated";
            };
            workloads.app = {
              providerRefs = {
                runtime = "runtime";
                device = "devices";
              };
              usbip.enable = enable;
            };
          };
        })
      ];
    };

  disabled = evalUsbip false;
  enabled = evalUsbip true;
  row = builtins.head enabled.config.d2b._index.devices.list;
  request = builtins.head
    enabled.config.d2b._index.devices.allocatorLeaseRequests;
in
{
  "usbip-gating/default-off" = {
    expr = disabled.config.d2b._index.devices.list;
    expected = [ ];
  };

  "usbip-gating/enabled-resource" = {
    expr = {
      inherit (row) resourceKind roleKind capability;
      attachment = row.mediation.attachment;
    };
    expected = {
      resourceKind = "usbip";
      roleKind = "usbip";
      capability = "usbip-exclusive";
      attachment = "fd-only";
    };
  };

  "usbip-gating/allocator-lease-request" = {
    expr = {
      inherit (request) resourceId kind share;
      phase = request.acquisitionOrder.phase;
      sourceKind = request.source.kind;
      sourceUsesProviderId = request.source.refName == row.providerId;
    };
    expected = {
      resourceId = "lease-device-security-key-global";
      kind = "host-file-partition";
      share = "exclusive";
      phase = 50;
      sourceKind = "realm-broker";
      sourceUsesProviderId = true;
    };
  };

  "usbip-gating/no-bus-id-path" = {
    expr =
      row.endpointId == null
      && !(row ? endpointPath)
      && !(row ? selectorId);
    expected = true;
  };
}
