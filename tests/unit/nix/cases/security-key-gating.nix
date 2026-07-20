{ flakeRoot, lib, ... }:

let
  evalDevices = workloads:
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
          config.d2b.realms.local-root = {
            path = "local-root";
            providers.runtime = {
              type = "runtime";
              implementationId = "cloud-hypervisor";
            };
            providers.devices = {
              type = "device";
              implementationId = "host-mediated";
            };
            inherit workloads;
          };
        })
      ];
    };

  enabled = evalDevices {
    corp = {
      providerRefs = {
        runtime = "runtime";
        device = "devices";
      };
      securityKey.enable = true;
    };
  };
  disabled = evalDevices {
    corp = {
      providerRefs.runtime = "runtime";
    };
  };
  row = builtins.head enabled.config.d2b._index.devices.list;
  providerFragment = import
    (flakeRoot + "/nixos-modules/provider-registry-v2-extensions/device.nix")
    {
      cfg = enabled.config.d2b;
      inherit lib;
    };
  provider = builtins.head providerFragment.providers;
in
{
  "security-key-gating/default-resource-absent" = {
    expr = disabled.config.d2b._index.devices.list;
    expected = [ ];
  };

  "security-key-gating/enabled-fido-resource" = {
    expr = {
      inherit (row) resourceKind capability roleKind;
      inherit (row.mediation) authority attachment broker;
    };
    expected = {
      resourceKind = "fido";
      capability = "fido-ceremony";
      roleKind = "security-key-frontend";
      authority = "host-mediated";
      attachment = "fd-only";
      broker = "realm-local";
    };
  };

  "security-key-gating/canonical-short-identities" = {
    expr = lib.all
      (value: builtins.match "[a-z2-7]{20}" value != null)
      [ row.realmId row.workloadId row.providerId row.roleId ];
    expected = true;
  };

  "security-key-gating/canonical-endpoint" = {
    expr = {
      inherit (row) endpointId;
      isCanonicalToken =
        lib.hasPrefix "device-endpoint-" row.endpointId
        && !(lib.hasInfix "/" row.endpointId)
        && !(lib.hasInfix ":" row.endpointId);
    };
    expected = {
      endpointId = "device-endpoint-${row.roleId}-fido";
      isCanonicalToken = true;
    };
  };

  "security-key-gating/provider-fragment" = {
    expr = {
      authority = provider.descriptor.authority.type;
      implementation = provider.descriptor.implementationId;
      capabilities = provider.descriptor.capabilities;
      axis = provider.binding.axis;
      resources = provider.binding.deviceResourceIds;
    };
    expected = {
      authority = "device";
      implementation = "host-mediated";
      capabilities = [
        "device.plan-attach"
        "device.attach"
        "device.inspect"
        "device.adopt"
        "device.detach"
      ];
      axis = "local-device";
      resources = [ row.resourceId ];
    };
  };

  "security-key-gating/no-physical-selector-in-path" = {
    expr =
      let serialized = builtins.toJSON row;
      in
      !(row ? selectorId)
      && !(row ? endpointPath)
      && !(lib.hasInfix "1050" serialized)
      && !(lib.hasInfix "0407" serialized)
      && !(lib.hasInfix "hidraw" serialized);
    expected = true;
  };
}
