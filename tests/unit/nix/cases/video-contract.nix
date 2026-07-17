{ flakeRoot, lib, system, ... }:

lib.optionalAttrs (system == "x86_64-linux") (
let
  evaluated = lib.evalModules {
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
          workloads.desktop = {
            provider = "runtime";
            launcher.capabilities = [ "gpu" "video" ];
          };
        };
      })
    ];
  };
  videoOnly = lib.evalModules {
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
          workloads.desktop = {
            provider = "runtime";
            launcher.capabilities = [ "video" ];
          };
        };
      })
    ];
  };
  rows = evaluated.config.d2b._index.devices.list;
  byKind = lib.listToAttrs (map
    (row: {
      name = row.resourceKind;
      value = row;
    })
    rows);
  gpu = byKind.gpu;
  render = byKind.render-node;
  video = byKind.video;
  videoSource = builtins.readFile
    (flakeRoot + "/nixos-modules/components/video/guest.nix");
in
{
  "video-contract/device-kinds" = {
    expr = map (row: row.resourceKind) rows;
    expected = [ "gpu" "render-node" "video" ];
  };

  "video-contract/canonical-role-kinds" = {
    expr = {
      gpu = gpu.roleKind;
      render = render.roleKind;
      video = video.roleKind;
    };
    expected = {
      gpu = "gpu";
      render = "gpu-render-node";
      video = "video";
    };
  };

  "video-contract/distinct-role-ids" = {
    expr = builtins.length
      (lib.unique [ gpu.roleId render.roleId video.roleId ]);
    expected = 3;
  };

  "video-contract/shared-render-node-leases" = {
    expr = map (row: row.allocatorLease) rows;
    expected = [
      { resourceId = "device-render-node-global"; share = "shared-partition"; }
      { resourceId = "device-render-node-global"; share = "shared-partition"; }
      { resourceId = "device-render-node-global"; share = "shared-partition"; }
    ];
  };

  "video-contract/canonical-video-socket" = {
    expr = video.endpointPath;
    expected =
      "/run/d2b/r/${video.realmId}/w/${video.workloadId}/roles/${video.roleId}/video.sock";
  };

  "video-contract/guest-uses-canonical-video-socket" = {
    expr =
      lib.hasInfix "/run/d2b/r/\${d2bRealmId}/w/\${d2bWorkloadId}/roles/\${d2bRoleIds.video}/video.sock"
        videoSource
      && !(lib.hasInfix "/run/d2b-video/" videoSource);
    expected = true;
  };

  "video-contract/fd-only-mediation" = {
    expr = lib.all
      (row: row.mediation.attachment == "fd-only")
      rows;
    expected = true;
  };

  "video-contract/video-requires-gpu" = {
    expr = lib.any
      (assertion:
        !assertion.assertion
        && lib.hasInfix "without GPU mediation" assertion.message)
      videoOnly.config.assertions;
    expected = true;
  };
}
)
