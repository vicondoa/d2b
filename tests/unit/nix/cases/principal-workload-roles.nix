{ lib, flakeRoot, ... }:

# Evaluated coverage for host-users.nix's workload/role-scoped sidecar
# principals (ADR 0045 W7fu17 H1). Proves that the eight distinct
# per-workload host account categories the framework's minijail/process/
# ownership contracts name (gpu, video, wayland-proxy, audio, swtpm,
# cloud-hypervisor runner, qemu-media runner, narrow guest-control fs) are
# created for enabled canonical workloads that declare the matching role,
# with the exact `d2b-role-<roleId>` / `d2b-gctlfs-<workloadId>` short-id
# principal names role-process-rows.nix / minijail-profiles.nix derive
# their sandbox uid/gid from -- and that every other role kind (the
# fake-rooted `gpu-render-node`, the generic `virtiofsd` share role, the
# always-present `guest-control-health` / `store-virtiofs-preflight` /
# `swtpm-pre-start-flush` / `vsock-relay` roles), a disabled workload, and
# a qemu-media workload's lack of a virtiofsd role all correctly produce
# NO host account.

let
  identity = import (flakeRoot + "/nixos-modules/v2-identity.nix");
  stablePrincipalId =
    (import (flakeRoot + "/nixos-modules/lib.nix") { inherit lib; }).stablePrincipalId;

  realmId = identity.deriveRealmId "lab.local-root";
  workloadId = name: identity.deriveWorkloadId realmId name;
  plainId = workloadId "plain";
  gfxId = workloadId "gfx";
  mediaId = workloadId "media";
  archivedId = workloadId "archived";

  roleId = wlId: roleKind: identity.deriveRoleId realmId wlId roleKind;
  rolePrincipal = wlId: roleKind: "d2b-role-${roleId wlId roleKind}";
  gctlfsPrincipal = wlId: "d2b-gctlfs-${wlId}";

  optionFixture =
    { lib, ... }:
    {
      options = {
        assertions = lib.mkOption {
          type = lib.types.listOf lib.types.attrs;
          default = [ ];
        };
        d2b.site = {
          launcherUsers = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
          };
          adminUsers = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
          };
        };
        users.groups = lib.mkOption {
          type = lib.types.attrsOf (
            lib.types.submodule {
              options.gid = lib.mkOption {
                type = lib.types.nullOr lib.types.int;
                default = null;
              };
            }
          );
          default = { };
        };
        users.users = lib.mkOption {
          type = lib.types.attrsOf (
            lib.types.submodule {
              options = {
                uid = lib.mkOption {
                  type = lib.types.nullOr lib.types.int;
                  default = null;
                };
                isSystemUser = lib.mkOption {
                  type = lib.types.bool;
                  default = false;
                };
                isNormalUser = lib.mkOption {
                  type = lib.types.bool;
                  default = false;
                };
                group = lib.mkOption {
                  type = lib.types.nullOr lib.types.str;
                  default = null;
                };
                extraGroups = lib.mkOption {
                  type = lib.types.listOf lib.types.str;
                  default = [ ];
                };
                description = lib.mkOption {
                  type = lib.types.str;
                  default = "";
                };
              };
            }
          );
          default = { };
        };
      };

      config = {
        d2b.realms.lab = {
          path = "lab.local-root";
          placement = "host-local";
          providers.runtime = {
            type = "runtime";
            implementationId = "cloud-hypervisor";
          };
          providers.media = {
            type = "runtime";
            implementationId = "qemu-media";
          };
          providers.display = {
            type = "display";
            implementationId = "wayland";
          };

          # cloud-hypervisor workload exercising every optional sidecar
          # category at once: gpu, video, audio, swtpm, wayland-proxy.
          workloads.gfx = {
            providerRefs.runtime = "runtime";
            providerRefs.display = "display";
            display.wayland = true;
            graphics.enable = true;
            graphics.videoSidecar = true;
            audio.enable = true;
            tpm.enable = true;
          };

          # cloud-hypervisor workload with no optional features: proves
          # the runner + narrow guest-control categories exist alone, and
          # that the optional categories stay absent when not requested.
          workloads.plain = {
            providerRefs.runtime = "runtime";
          };

          # qemu-media workload: proves the distinct qemu-media runner
          # category, and that qemu-media never gets a virtiofsd role
          # (so no narrow guest-control principal is created for it).
          workloads.media = {
            providerRefs.runtime = "media";
          };

          # Disabled workload requesting graphics: proves a disabled
          # workload contributes zero host principals at all, even though
          # the underlying normalized role index still computes its
          # (unused) role rows.
          workloads.archived = {
            enable = false;
            providerRefs.runtime = "runtime";
            graphics.enable = true;
          };
        };
      };
    };

  evaluated = lib.evalModules {
    modules = [
      optionFixture
      (flakeRoot + "/nixos-modules/options-realms.nix")
      (flakeRoot + "/nixos-modules/index.nix")
      (flakeRoot + "/nixos-modules/host-users.nix")
    ];
  };
  cfg = evaluated.config;
  failedAssertions = lib.filter (entry: !entry.assertion) cfg.assertions;

  hasGroup = name: builtins.hasAttr name cfg.users.groups;
  hasUser = name: builtins.hasAttr name cfg.users.users;
  groupIdOf = name: cfg.users.groups.${name}.gid;
  userOf = name: cfg.users.users.${name};
in
{
  "principal-workload-roles/module-assertions-hold" = {
    expr = failedAssertions;
    expected = [ ];
  };

  "principal-workload-roles/eight-categories-present-for-enabled-workloads" = {
    expr = {
      gpu = hasGroup (rolePrincipal gfxId "gpu") && hasUser (rolePrincipal gfxId "gpu");
      video = hasGroup (rolePrincipal gfxId "video") && hasUser (rolePrincipal gfxId "video");
      waylandProxy =
        hasGroup (rolePrincipal gfxId "wayland-proxy")
        && hasUser (rolePrincipal gfxId "wayland-proxy");
      audio = hasGroup (rolePrincipal gfxId "audio") && hasUser (rolePrincipal gfxId "audio");
      swtpm = hasGroup (rolePrincipal gfxId "swtpm") && hasUser (rolePrincipal gfxId "swtpm");
      runner =
        hasGroup (rolePrincipal plainId "cloud-hypervisor")
        && hasUser (rolePrincipal plainId "cloud-hypervisor");
      qemuMedia =
        hasGroup (rolePrincipal mediaId "qemu-media")
        && hasUser (rolePrincipal mediaId "qemu-media");
      narrowGuestControl =
        hasGroup (gctlfsPrincipal plainId) && hasUser (gctlfsPrincipal plainId);
    };
    expected = {
      gpu = true;
      video = true;
      waylandProxy = true;
      audio = true;
      swtpm = true;
      runner = true;
      qemuMedia = true;
      narrowGuestControl = true;
    };
  };

  "principal-workload-roles/principal-ids-are-stable-and-symmetric-uid-gid" = {
    expr = {
      gpuUid = (userOf (rolePrincipal gfxId "gpu")).uid;
      gpuGid = groupIdOf (rolePrincipal gfxId "gpu");
      gpuGroupField = (userOf (rolePrincipal gfxId "gpu")).group;
      runnerUid = (userOf (rolePrincipal plainId "cloud-hypervisor")).uid;
      runnerGid = groupIdOf (rolePrincipal plainId "cloud-hypervisor");
      gctlfsUid = (userOf (gctlfsPrincipal plainId)).uid;
      gctlfsGid = groupIdOf (gctlfsPrincipal plainId);
    };
    expected = {
      gpuUid = stablePrincipalId (rolePrincipal gfxId "gpu");
      gpuGid = stablePrincipalId (rolePrincipal gfxId "gpu");
      gpuGroupField = rolePrincipal gfxId "gpu";
      runnerUid = stablePrincipalId (rolePrincipal plainId "cloud-hypervisor");
      runnerGid = stablePrincipalId (rolePrincipal plainId "cloud-hypervisor");
      gctlfsUid = stablePrincipalId (gctlfsPrincipal plainId);
      gctlfsGid = stablePrincipalId (gctlfsPrincipal plainId);
    };
  };

  "principal-workload-roles/gpu-and-audio-extra-groups" = {
    expr = {
      gpuExtraGroups = lib.sort lib.lessThan (userOf (rolePrincipal gfxId "gpu")).extraGroups;
      audioExtraGroups = (userOf (rolePrincipal gfxId "audio")).extraGroups;
    };
    expected = {
      gpuExtraGroups = lib.sort lib.lessThan [
        "kvm"
        (rolePrincipal gfxId "cloud-hypervisor")
      ];
      audioExtraGroups = [ "audio" ];
    };
  };

  "principal-workload-roles/non-principal-role-kinds-produce-no-account" = {
    expr = {
      gpuRenderNode =
        hasGroup (rolePrincipal gfxId "gpu-render-node")
        || hasUser (rolePrincipal gfxId "gpu-render-node");
      guestControlHealth =
        hasGroup (rolePrincipal plainId "guest-control-health")
        || hasUser (rolePrincipal plainId "guest-control-health");
      storeVirtiofsPreflight =
        hasGroup (rolePrincipal plainId "store-virtiofs-preflight")
        || hasUser (rolePrincipal plainId "store-virtiofs-preflight");
      swtpmPreStartFlush =
        hasGroup (rolePrincipal gfxId "swtpm-pre-start-flush")
        || hasUser (rolePrincipal gfxId "swtpm-pre-start-flush");
      vsockRelay =
        hasGroup (rolePrincipal plainId "vsock-relay")
        || hasUser (rolePrincipal plainId "vsock-relay");
      bareVirtiofsd =
        hasGroup (rolePrincipal plainId "virtiofsd")
        || hasUser (rolePrincipal plainId "virtiofsd");
    };
    expected = {
      gpuRenderNode = false;
      guestControlHealth = false;
      storeVirtiofsPreflight = false;
      swtpmPreStartFlush = false;
      vsockRelay = false;
      bareVirtiofsd = false;
    };
  };

  "principal-workload-roles/qemu-media-workload-has-no-narrow-guest-control" = {
    expr = hasGroup (gctlfsPrincipal mediaId) || hasUser (gctlfsPrincipal mediaId);
    expected = false;
  };

  "principal-workload-roles/disabled-workload-has-zero-principals" = {
    expr = lib.any
      (roleKind:
        hasGroup (rolePrincipal archivedId roleKind)
        || hasUser (rolePrincipal archivedId roleKind))
      [ "gpu" "gpu-render-node" "video" "wayland-proxy" "audio" "swtpm"
        "swtpm-pre-start-flush" "cloud-hypervisor" "guest-control-health"
        "store-virtiofs-preflight" "vsock-relay" "virtiofsd" ]
    || hasGroup (gctlfsPrincipal archivedId)
    || hasUser (gctlfsPrincipal archivedId);
    expected = false;
  };

  "principal-workload-roles/distinct-principal-count" = {
    expr =
      let
        prefixed = lib.filter
          (name: lib.hasPrefix "d2b-role-" name || lib.hasPrefix "d2b-gctlfs-" name)
          (builtins.attrNames cfg.users.groups);
        ids = map stablePrincipalId prefixed;
      in
      {
        groupCount = builtins.length prefixed;
        distinctNames = builtins.length (lib.unique prefixed);
        distinctIds = builtins.length (lib.unique ids);
      };
    expected = {
      # plain: cloud-hypervisor runner + narrow guest-control (2)
      # gfx: gpu, video, audio, swtpm, wayland-proxy, cloud-hypervisor
      #      runner + narrow guest-control (7)
      # media: qemu-media runner (1)
      groupCount = 10;
      distinctNames = 10;
      distinctIds = 10;
    };
  };
}
