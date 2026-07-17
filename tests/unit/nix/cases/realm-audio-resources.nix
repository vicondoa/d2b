{ flakeRoot, lib, pkgs, ... }:

let
  evaluation = lib.evalModules {
    modules = [
      (flakeRoot + "/nixos-modules/index.nix")
      (flakeRoot + "/nixos-modules/minijail-profiles.nix")
      ({ lib, ... }: {
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
  roleProfile =
    config.d2b._bundle.minijailProfiles."role-${process.roleId}".roleProfile;
  canonicalArgvPrefix = "# canonical-argv: ";
  goldenLines = lib.splitString "\n"
    (builtins.readFile
      (flakeRoot + "/tests/golden/runner-shape/audio-argv-minimal.txt"));
  canonicalArgvLine = lib.findFirst
    (line: lib.hasPrefix canonicalArgvPrefix line)
    (throw "audio argv golden is missing its canonical realm-role payload")
    goldenLines;
  goldenArgv = lib.splitString " "
    (lib.removePrefix canonicalArgvPrefix canonicalArgvLine);
  serialized = builtins.toJSON {
    inherit (rows) processes endpoints storage leases;
    inherit (fragment) providers;
    inherit roleProfile;
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
      inherit (process)
        kind
        roleId
        supervision
        cgroupPlacement
        seccompPolicyRef
        startAfterLeaseIds
        ;
      argv = process.argv;
      inherit (process) environment;
      goldenMatches = process.argv == goldenArgv;
      endpoint = {
        inherit (endpoint)
          kind
          transport
          path
          mode
          lifecycle
          ownerRoleId
          peerRoleIds
          listenerOwner
          ;
      };
      state = {
        inherit (state) path mode maxBytes initialState atomicReplace;
      };
      lease = {
        inherit (lease)
          kind
          share
          source
          delivery
          acquisitionOrder
          revocation
          ;
      };
      profile = {
        inherit (roleProfile)
          caps
          seccompPolicyRef
          namespaces
          userNamespace
          umask
          ;
        readOnlyPaths = roleProfile.mountPolicy.readOnlyPaths;
        writablePaths = map (row: row.path) roleProfile.mountPolicy.writablePaths;
        inherit (roleProfile.mountPolicy)
          deviceBinds
          bindMounts
          nixStoreReadOnly
          hideDeviceNodesByDefault
          ;
        cgroup = roleProfile.cgroupPlacement.subtree;
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
      seccompPolicyRef = "w1-audio";
      startAfterLeaseIds = [ "audio-pipewire-3ktvlfsdkkqlcugirbcq" ];
      argv = [
        "/run/d2b/r/tft6a4n527flrfmxjwna/w/3ktvlfsdkkqlcugirbcq/roles/dfjudgner53qnwyowkja/d2b-audio-3ktvlfsdkkqlcugirbcq"
        "--socket"
        "/run/d2b/r/tft6a4n527flrfmxjwna/w/3ktvlfsdkkqlcugirbcq/sockets/audio.sock"
        "--backend"
        "pipewire"
      ];
      environment = [
        "PIPEWIRE_RUNTIME_DIR=/run/d2b/r/tft6a4n527flrfmxjwna/w/3ktvlfsdkkqlcugirbcq/roles/dfjudgner53qnwyowkja/pipewire"
        "XDG_RUNTIME_DIR=/run/d2b/r/tft6a4n527flrfmxjwna/w/3ktvlfsdkkqlcugirbcq/roles/dfjudgner53qnwyowkja/pipewire"
        ''PIPEWIRE_PROPS={ application.name = "d2b-3ktvlfsdkkqlcugirbcq" node.name = "d2b-3ktvlfsdkkqlcugirbcq" node.description = "d2b 3ktvlfsdkkqlcugirbcq" }''
      ];
      goldenMatches = true;
      endpoint = {
        kind = "vhost-user-sound";
        transport = "unix-stream";
        path =
          "/run/d2b/r/tft6a4n527flrfmxjwna/w/3ktvlfsdkkqlcugirbcq/sockets/audio.sock";
        mode = "0660";
        lifecycle = "workload";
        ownerRoleId = "dfjudgner53qnwyowkja";
        peerRoleIds = [ "asw7f5tc7jk6hki54ava" ];
        listenerOwner = "audio-role";
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
        source = {
          kind = "active-host-audio-session";
          refName = "pipewire";
        };
        delivery = {
          kind = "bind-single-endpoint";
          targetStorageRef = "audio-runtime-dfjudgner53qnwyowkja";
          targetRelativePath = "pipewire-0";
          parentRuntimeVisible = false;
        };
        acquisitionOrder = {
          phase = 45;
          ordinal = 0;
        };
        revocation = "workload-stop";
      };
      profile = {
        caps = [ ];
        seccompPolicyRef = "w1-audio";
        namespaces = {
          ipc = true;
          mount = true;
          net = false;
          pid = false;
          user = false;
          uts = false;
        };
        userNamespace = null;
        umask = 7;
        readOnlyPaths = [ "/nix/store" ];
        writablePaths = [
          "/run/d2b/r/tft6a4n527flrfmxjwna/w/3ktvlfsdkkqlcugirbcq/sockets"
          "/run/d2b/r/tft6a4n527flrfmxjwna/w/3ktvlfsdkkqlcugirbcq/roles/dfjudgner53qnwyowkja/pipewire"
        ];
        deviceBinds = [ ];
        bindMounts = [ ];
        nixStoreReadOnly = true;
        hideDeviceNodesByDefault = true;
        cgroup =
          "d2b.slice/r-tft6a4n527flrfmxjwna/workloads/w-3ktvlfsdkkqlcugirbcq/dfjudgner53qnwyowkja";
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
      builtins.match ".*(/run/user/|wayland-[0-9]|work[.]local-root|editor|alice|CAP_[A-Z_]+).*"
        serialized == null;
    expected = true;
  };
}
