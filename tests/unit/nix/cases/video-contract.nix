{ flakeRoot, lib, pkgs, system, ... }:

# Coverage owned by this file:
#   * device-kinds / canonical-role-kinds — gpu, gpu-render-node ("render-node"),
#     and video device rows are each derived with the roleKind the runtime
#     process/minijail/guest layers key off of.
#   * distinct-role-ids — every (workload, roleKind) pair gets its own
#     identity-derived roleId, even when two rows share a workload.
#   * shared-render-node-leases — gpu/render-node/video device rows all
#     acquire the same shared render-node allocator lease (never exclusive).
#   * canonical-video-socket / guest-uses-canonical-video-socket — the video
#     role's runtime resource path plus `/video.sock` matches exactly the
#     socket path the real guest wiring module (`components/video/guest.nix`)
#     computes when evaluated with the same realm/workload/role ids, so a
#     drift here is fail-visible from both sides of the Nix/guest boundary.
#   * fd-only-mediation — every device row stays fd-only (no path-based device
#     handoff to the guest).
#   * video-requires-gpu — requesting the video sidecar without GPU mediation
#     on the same workload is a hard eval-time assertion failure.
lib.optionalAttrs (system == "x86_64-linux") (
let
  mkFixture = workloads: lib.evalModules {
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
          inherit workloads;
        };
      })
    ];
  };
  # Two workloads so a single realm exercises all three device kinds: the
  # mutually-exclusive gpu/render-node choice (`graphics.renderNodeOnly`)
  # means one workload can never request both in the current contract.
  evaluated = mkFixture {
    desktop = {
      providerRefs = {
        runtime = "runtime";
        device = "devices";
      };
      graphics = {
        enable = true;
        videoSidecar = true;
      };
    };
    capture = {
      providerRefs = {
        runtime = "runtime";
        device = "devices";
      };
      graphics = {
        enable = true;
        renderNodeOnly = true;
      };
    };
  };
  videoOnly = mkFixture {
    desktop = {
      providerRefs = {
        runtime = "runtime";
        device = "devices";
      };
      graphics.videoSidecar = true;
    };
  };
  rows = evaluated.config.d2b._index.devices.list;
  byKind = lib.listToAttrs (map
    (row: {
      name = row.resourceKind;
      value = row;
    })
    rows);
  gpu = byKind.gpu;
  render = byKind."render-node";
  video = byKind.video;
  videoRoleResource =
    evaluated.config.d2b._index.resources.byId."role/${video.roleId}/runtime";
  # Evaluate the real guest wiring module with the exact realm/workload/role
  # ids the runtime side derived above, instead of scanning guest.nix's
  # source text: this proves the guest module's own computed socket path
  # actually matches the role resource, not merely that some string in the
  # file happens to look right.
  videoGuestModule = import (flakeRoot + "/nixos-modules/components/video/guest.nix") {
    # The video kernel module package is irrelevant to the socket-path
    # contract under test and is never forced below, so a stub avoids
    # pulling in a real kernel package build during eval.
    config.boot.kernelPackages.callPackage = _pkg: _args: null;
    inherit lib pkgs;
    d2bRealmId = video.realmId;
    d2bWorkloadId = video.workloadId;
    d2bRoleIds = { video = video.roleId; };
  };
  videoGuestArgs =
    videoGuestModule.microvm.cloud-hypervisor.extraArgs.content;
  videoGuestSocket =
    lib.removePrefix "socket=" (lib.last videoGuestArgs);
in
{
  "video-contract/device-kinds" = {
    expr = lib.sort lib.lessThan (map (row: row.resourceKind) rows);
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
    expr = lib.all
      (row:
        row.allocatorLeaseId == "lease-device-render-node-global"
        && row.allocatorShare == "shared-partition")
      rows;
    expected = true;
  };

  "video-contract/canonical-video-socket" = {
    expr = videoRoleResource.path + "/video.sock";
    expected =
      "/run/d2b/r/${video.realmId}/w/${video.workloadId}/roles/${video.roleId}/video.sock";
  };

  "video-contract/guest-uses-canonical-video-socket" = {
    expr = videoGuestSocket;
    expected = videoRoleResource.path + "/video.sock";
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
