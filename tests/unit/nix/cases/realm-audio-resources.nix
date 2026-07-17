{ flakeRoot, lib, pkgs, ... }:

let
  evaluation = lib.evalModules {
    modules = [
      (flakeRoot + "/nixos-modules/index.nix")
      ({ lib, ... }: {
        options.d2b.realms = lib.mkOption {
          type = lib.types.attrs;
          default = { };
        };
        config.d2b.realms = {
          work = {
            path = "work.local-root";
            placement = "host-local";
            providers.runtime = {
              primaryAuthority = "runtime";
              implementation = "cloud-hypervisor";
            };
            workloads = {
              editor = {
                runtime.implementation = "cloud-hypervisor";
                audio = {
                  enable = true;
                  allowMicByDefault = false;
                  allowSpeakerByDefault = true;
                };
              };
              quiet = {
                runtime.implementation = "cloud-hypervisor";
                audio.enable = false;
              };
            };
          };
        };
      })
    ];
  };
  config = evaluation.config;
  rows = import
    "${flakeRoot}/nixos-modules/realm-audio-rows.nix"
    { inherit config lib pkgs; };
  fragment = import
    "${flakeRoot}/nixos-modules/provider-registry-v2-extensions/audio.nix"
    { inherit config lib pkgs; generation = 7; };
  process = builtins.head rows.processes;
  endpoint = builtins.head rows.endpoints;
  state = lib.findFirst
    (row: row.kind == "audio-policy-state")
    null
    rows.storage;
  lease = builtins.head rows.leases;
  provider = builtins.head fragment.providers;
  serialized = builtins.toJSON {
    inherit (rows) processes endpoints storage leases;
    inherit (fragment) providers;
  };
in
{
  "realm-audio/resources-use-canonical-short-id-paths" = {
    expr = {
      count = {
        processes = builtins.length rows.processes;
        endpoints = builtins.length rows.endpoints;
        storage = builtins.length rows.storage;
        leases = builtins.length rows.leases;
      };
      inherit (process) kind roleId supervision cgroupPlacement;
      argv = builtins.tail process.argv;
      endpoint = {
        inherit (endpoint) kind transport path mode lifecycle;
      };
      state = {
        inherit (state) path mode maxBytes initialState atomicReplace;
      };
      lease = {
        inherit (lease) kind share delivery;
      };
    };
    expected = {
      count = {
        processes = 1;
        endpoints = 1;
        storage = 3;
        leases = 1;
      };
      kind = "vhost-user-sound";
      roleId = "dfjudgner53qnwyowkja";
      supervision = "realm-controller-pidfd";
      cgroupPlacement = "direct-role-leaf";
      argv = [
        "--socket"
        "/run/d2b/r/tft6a4n527flrfmxjwna/w/3ktvlfsdkkqlcugirbcq/sockets/audio.sock"
        "--backend"
        "pipewire"
      ];
      endpoint = {
        kind = "vhost-user-sound";
        transport = "unix-stream";
        path =
          "/run/d2b/r/tft6a4n527flrfmxjwna/w/3ktvlfsdkkqlcugirbcq/sockets/audio.sock";
        mode = "0660";
        lifecycle = "workload";
      };
      state = {
        path =
          "/var/lib/d2b/r/tft6a4n527flrfmxjwna/w/3ktvlfsdkkqlcugirbcq/audio/audio-state.json";
        mode = "0640";
        maxBytes = 128;
        initialState = {
          mic = "off";
          speaker = "on";
        };
        atomicReplace = true;
      };
      lease = {
        kind = "pipewire-session-endpoint";
        share = "shared-partition";
        delivery = {
          kind = "bind-single-endpoint";
          targetStorageRef = "audio-runtime-dfjudgner53qnwyowkja";
          targetRelativePath = "pipewire-0";
          parentRuntimeVisible = false;
        };
      };
    };
  };

  "realm-audio/provider-fragment-is-closed" = {
    expr = {
      inherit (fragment) axis generation;
      inherit (provider.descriptor)
        authority
        implementationId
        capabilities
        registryGeneration
        placement
        ;
      providerIdLength = builtins.stringLength provider.descriptor.providerId;
      fingerprintLength =
        builtins.stringLength provider.descriptor.configurationSchemaFingerprint;
      digestLength =
        builtins.stringLength provider.descriptor.configuredScopeDigest;
      inherit (provider) binding;
    };
    expected = {
      axis = "audio";
      generation = 7;
      authority.type = "audio";
      implementationId = "pipewire-vhost-user";
      capabilities = [
        "audio.open"
        "audio.set-state"
        "audio.inspect"
        "audio.adopt"
        "audio.close"
      ];
      registryGeneration = 7;
      placement = {
        kind = "trusted-first-party-in-process";
        realmId = "tft6a4n527flrfmxjwna";
        controllerRole = "realm-controller";
      };
      providerIdLength = 20;
      fingerprintLength = 64;
      digestLength = 64;
      binding = {
        axis = "local-audio";
        workloadId = "3ktvlfsdkkqlcugirbcq";
        roleId = "dfjudgner53qnwyowkja";
        processId = "audio-process-dfjudgner53qnwyowkja";
        endpointId = "audio-vhost-3ktvlfsdkkqlcugirbcq";
        stateStorageId = "audio-state-3ktvlfsdkkqlcugirbcq";
        lockStorageId = "audio-lock-3ktvlfsdkkqlcugirbcq";
        mediationStorageId = "audio-runtime-dfjudgner53qnwyowkja";
        leaseId = "audio-pipewire-3ktvlfsdkkqlcugirbcq";
      };
    };
  };

  "realm-audio/bundle-rows-exclude-ambient-host-endpoints" = {
    expr =
      builtins.match ".*(/run/user/|wayland-[0-9]|work[.]local-root|editor).*"
        serialized == null;
    expected = true;
  };
}
