{ flakeRoot, lib, system, ... }:

# Coverage owned by this file: the per-role minijail profile's rendered
# `umask` field for the four roles that must own their runtime/socket paths
# exclusively (0700 via umask 7) once granted a private mount namespace:
# swtpm, gpu, video, and audio. Each case evaluates the real
# index.nix + minijail-profiles.nix pipeline over a fixture workload that
# enables all four roles and reads the actual rendered
# `_bundle.minijailProfiles."role-${roleId}".data.umask`, rather than
# scanning minijail-profiles.nix source text.
lib.optionalAttrs (system == "x86_64-linux") (
  let
    fixtureModule = { lib, ... }: {
      options = {
        assertions = lib.mkOption {
          type = lib.types.listOf lib.types.attrs;
          default = [ ];
        };
        environment.etc = lib.mkOption {
          type = lib.types.attrs;
          default = { };
        };
        d2b = {
          realms = lib.mkOption {
            type = lib.types.attrs;
            default = { };
          };
          _bundle.minijailProfiles = lib.mkOption {
            type = lib.types.attrs;
            default = { };
          };
        };
      };
      config.d2b.realms.work = {
        path = "work.local-root";
        providers.runtime = {
          type = "runtime";
          implementationId = "cloud-hypervisor";
        };
        providers.audio = {
          type = "audio";
          implementationId = "pipewire-vhost-user";
        };
        workloads.desktop = {
          providerRefs = {
            runtime = "runtime";
            audio = "audio";
          };
          tpm.enable = true;
          graphics = {
            enable = true;
            videoSidecar = true;
          };
          audio.enable = true;
          # The graphics/audio enablement above would otherwise default
          # `display.wayland` on and add a wayland-proxy role that needs
          # its own display provider binding; this fixture only exercises
          # the swtpm/gpu/video/audio umask, so keep it out of scope.
          display.wayland = false;
        };
      };
    };
    evaluation = lib.evalModules {
      modules = [
        (flakeRoot + "/nixos-modules/index.nix")
        (flakeRoot + "/nixos-modules/minijail-profiles.nix")
        fixtureModule
      ];
    };
    config = evaluation.config;
    workload =
      config.d2b._index.workloads.byCanonicalTarget."desktop.work.local-root.d2b";
    roleIdFor = roleKind:
      (lib.findFirst
        (role: role.roleKind == roleKind)
        (throw "umask-roundtrip: workload desktop is missing role ${roleKind}")
        workload.roles).roleId;
    umaskFor = roleKind:
      config.d2b._bundle.minijailProfiles."role-${roleIdFor roleKind}".data.umask;
  in
  {
    "umask-roundtrip/swtpm" = {
      expr = umaskFor "swtpm";
      expected = 7;
    };
    "umask-roundtrip/gpu" = {
      expr = umaskFor "gpu";
      expected = 7;
    };
    "umask-roundtrip/video" = {
      expr = umaskFor "video";
      expected = 7;
    };
    "umask-roundtrip/audio" = {
      expr = umaskFor "audio";
      expected = 7;
    };
  }
)
