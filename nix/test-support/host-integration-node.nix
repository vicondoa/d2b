# Guest node configuration for the d2b host-integration lane.
#
# This is the reusable half of the VM fixtures' shared node: the module
# every d2b daemon-host check boots, plus the per-check module contribution
# that rides on top of it. It lives here rather than under
# `tests/host-integration/` because that directory is removed check by check
# as each one ports; the guest a check needs has to outlive the fixture that
# used to declare it.
#
# The lane evaluates this module directly, carrying the same QEMU VM module
# the runNixOSTest nodes carry, and reads each check's invocation back off
# the evaluated configuration: `virtualisation.{memorySize,cores,diskSize}`,
# `virtualisation.useBootLoader`, `virtualisation.qemu.drives`, and
# `virtualisation.qemu.options`. Every one of those fields is declared here
# exactly once, so a guest is reproduced from its own declaration rather than
# from one uniform shape. A per-check module passed as `extra` merges through
# the same import the fixture used, so its own memory, drive, and device
# declarations - a vsock device, say - reach the same options list the shape
# contributes to.
#
# Two shapes, both named here and neither collapsed into the other:
#
#   d2bDaemonNode          the default node. /var/lib/d2b rides a dedicated
#                          state disk attached through `qemu.options`, and the
#                          root image keeps the VM module's own writeback
#                          cache.
#   d2bCloudHypervisorNode the writable-store node. /nix/store and
#                          /var/lib/d2b must share one filesystem for the
#                          hardlink farm, so the state disk is dropped, the
#                          root drive replaces it with an unsafe cache, and
#                          the node boots through a bootloader.
#
# A check whose assertions have been ported to Rust has no fixture left to
# declare its guest in, so it is named in `portedCheckNodes` at the bottom of
# this file instead: one entry per ported check, built from one of the shapes
# above. The two ways of declaring a guest are the same declaration - the node
# module - read from the two places a check can be written down. `shapeNodes`
# names the two reusable shapes themselves, which is also how a fixture-less
# image that is neither a shape nor a ported check is refused rather than
# quietly served the default node.
{ self, lib }:

let
  # The minimal, hermetic d2b site declaration every daemon-host node shares.
  # Zone and Guest resources belong to the acceptance fixture that exercises
  # them; keeping this base free of legacy VM/env authoring prevents unrelated
  # host checks from silently materializing a second lifecycle graph.
  daemonAcceptanceUnits = [
    "d2bd.service"
    "d2b-broker.socket"
    "d2b-broker.service"
  ];

  baseD2bConfig = {
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
      usePrebuiltHostTools = false;
    };
    # The daemon's v3 bundle always carries the local-root storage row. Keep
    # the corresponding root Zone in these minimal host fixtures so the
    # emitted topology is sealed and the daemon can enter Ready.
    d2b.zones.local-root = { };
    # The full daemon + broker systemd surface under test.
    d2b.daemonExperimental.enable = true;
  };
in
rec {
  # A NixOS module for a runNixOSTest node that boots the d2b daemon host.
  # `extra` is merged as an additional module so individual tests can add
  # per-test Zone/Guest resources, tampering helpers, or a larger disk. The
  # node provisions the `alice` operator user the base config references.
  #
  # Structured as an attrset-module with everything in `imports` (an attrset is
  # a valid module): `imports` must be top-level, NOT wrapped in `lib.mkMerge`,
  # or the module system rejects it ("option nodes.machine.imports does not
  # exist").
  d2bDaemonNode =
    { extra ? { }, writableStore ? false }:
    { config, pkgs, ... }:
    let
      # Dedicated state disk: the redb Zone store pays an fsync per commit
      # on the emulated root disk, and bring-up write bursts stall the
      # daemon writer thread ~700-900ms per write. Attaching /var/lib/d2b
      # as its own virtio drive with cache=unsafe makes guest fsync a host
      # page-cache no-op; the fixture VM is ephemeral, so the durability
      # semantics that unsafe drops are irrelevant here.
      stateDisk = pkgs.runCommand "d2b-state.img"
        {
          nativeBuildInputs = [ pkgs.e2fsprogs ];
        }
        ''
          truncate -s 4G "$out"
          mkfs.ext4 -q -F "$out"
        '';
    in
    {
      imports = [
        self.nixosModules.default
        baseD2bConfig
        extra
        {
          # Headroom for building/activating the bundle + daemon closure inside
          # the VM; the default 1024 MiB is tight once the broker spawns
          # runners.
          virtualisation.memorySize = 3072;
          virtualisation.diskSize = 8192;
          # The daemon's redb writer thread, the daemon async runtime, the
          # broker, and three controllers all need real CPU. The 1-vCPU
          # default serializes them and starves every 250ms handshake.
          virtualisation.cores = 3;
          boot.kernelModules = [ "br_netfilter" "tun" "vhost_net" ];

          users.users.alice = {
            isNormalUser = true;
            uid = 1000;
          };

          environment.etc."d2b/daemon-acceptance-units".text =
            lib.concatStringsSep "\n" daemonAcceptanceUnits + "\n";

          # Fail VM checks promptly when daemon startup is deterministically
          # broken instead of spending the lane timeout in a restart loop.
          systemd.services.d2bd.unitConfig = {
            StartLimitIntervalSec = "30s";
            StartLimitBurst = 3;
          };

          # runNixOSTest runs first-boot activation before systemd-tmpfiles has
          # materialized the d2b state tree. Pre-create the state directory so
          # daemon-owned startup can rely on the same path ordering.
          system.activationScripts.d2bTestStateDirs = {
            deps = [ "users" ];
            text = ''
              install -d -m 0750 -o root -g d2bd /var/lib/d2b
              install -d -m 0710 -o root -g d2b /var/lib/d2b/keys
              : > /var/lib/d2b/keys/.lock
              chown root:root /var/lib/d2b/keys/.lock
              chmod 0600 /var/lib/d2b/keys/.lock
            '';
          };
          system.stateVersion = "25.11";
        }
        # Opt-in writable same-fs store. ONLY needed by tests that drive the
        # per-VM /nix/store hardlink farm (which requires /var/lib/d2b and
        # /nix/store on the SAME filesystem - hardlinks can't cross FS - and the
        # default runNixOSTest read-only store image splits them). It is OFF by
        # default: `virtualisation.writableStore = true` copies the entire guest
        # closure into a writable overlay at boot, which adds many minutes to
        # (and can hang) VM startup. The daemon/broker activation + host-posture
        # tests (daemon-smoke, bridge-isolation, privilege-oracle)
        # never boot a microVM, so they never touch the farm - keep this off for
        # a fast, reliable boot.
        (lib.mkIf writableStore {
          virtualisation.useBootLoader = true;
          # The guest store-view hardlinks /nix/store into /var/lib/d2b, so
          # both must stay on one filesystem - a separate state disk would
          # break the hardlink farm with EXDEV. Instead, drop the root
          # drive's cache to unsafe: every redb commit's fsync becomes a
          # host page-cache no-op instead of a ~700-900ms stall, and the
          # fixture VM is ephemeral, so the lost durability is irrelevant.
          virtualisation.qemu.drives = lib.mkForce [
            {
              name = "root";
              file = ''"$NIX_DISK_IMAGE"'';
              driveExtraOpts.cache = "unsafe";
              driveExtraOpts.werror = "report";
              deviceExtraOpts.bootindex = "1";
              deviceExtraOpts.serial = "root";
            }
          ];
        })
        # The state disk keeps /var/lib/d2b off the emulated root disk:
        # cache=unsafe (host fsync no-op), noatime + nobarrier mounts.
        # The writableStore hardlink-farm tests stay on the default
        # same-fs layout, so this is opt-out for them.
        (lib.mkIf (! writableStore) {
          # The image is a `pkgs.runCommand` output, so QEMU must not need
          # write access to it: the lane builds these checks inside the Nix
          # sandbox, where /nix/store is mounted read-only, and a writable
          # drive on a store path makes QEMU abort at machine start - the
          # test driver surfaces that as a bare "Connection reset by peer".
          # `snapshot=on` opens the backing file read-only and keeps every
          # guest write in an ephemeral per-VM overlay under TMPDIR, which
          # matches the fixture's ephemeral state disk either way.
          virtualisation.qemu.options = [
            "-drive"
            "file=${stateDisk},format=raw,if=virtio,cache=unsafe,aio=threads,snapshot=on"
          ];
          fileSystems."/var/lib/d2b" = {
            device = "/dev/vdb";
            fsType = "ext4";
            options = [ "noatime" "nobarrier" ];
            # Up before activation so d2bTestStateDirs lands inside the
            # mounted filesystem, not under the covered root mountpoint.
            neededForBoot = true;
          };
        })
      ];
    };

  # Shared host posture for every fixture that boots a Cloud Hypervisor Guest.
  # The hardlink-backed Guest store view requires a writable host store on the
  # same filesystem as /var/lib/d2b.
  d2bCloudHypervisorNode =
    { extra ? { } }:
    d2bDaemonNode {
      inherit extra;
      writableStore = true;
    };

  # The guest `daemon-smoke` boots: the reusable daemon node plus the JSON
  # reader its assertions read the daemon's answers with.
  #
  # The node is a plain module here rather than a shape constructor, because
  # there is nothing left to parameterise: the check's fixture declared
  # `d2bDaemonNode` with this one package on top, and the check's port retires
  # that fixture, so the declaration has to live where the reusable nodes do.
  d2bDaemonSmokeNode = d2bDaemonNode {
    extra = { pkgs, ... }: {
      environment.systemPackages = [ pkgs.jq ];
    };
  };

  # The guest `bridge-isolation` boots: a plain NixOS node with the two
  # userspace tools its assertions drive the bridge and its namespaces with.
  #
  # The check never wanted the d2b daemon host - it configures a bridge as
  # root inside the guest and asserts the kernel's port-isolation semantics
  # on it - so this node is not built on `d2bDaemonNode`. It declared these
  # two packages and its `stateVersion`; the machine size, the disk, and the
  # emulator invocation are the QEMU VM module's defaults, which the lane
  # reads back off the image's manifest rather than restating here.
  d2bBridgeIsolationNode = { pkgs, ... }: {
    environment.systemPackages = [
      pkgs.iproute2
      pkgs.iputils
    ];
    system.stateVersion = "25.11";
  };

  # The guest `guest-agent-cap-confinement` boots: a plain NixOS node with an
  # unprivileged network-agent user, an isolated network namespace, and the
  # agent process confined to it.
  #
  # As with `bridge-isolation`, the check never wanted the d2b daemon host,
  # so this node is not built on `d2bDaemonNode`. The two units are the
  # fixture's own: the namespace unit creates `/run/netns/d2b-test-agent`,
  # and the agent unit runs as the unprivileged user inside that namespace
  # with the three capabilities the check asserts on, no ambient privilege
  # beyond them, and `NoNewPrivileges`. Moving them here is what lets the
  # fixture go without the guest the check asserts against going with it.
  d2bGuestAgentCapConfinementNode = { pkgs, ... }: {
    users.groups.d2b-net-agent-test = { };
    users.users.d2b-net-agent-test = {
      isSystemUser = true;
      group = "d2b-net-agent-test";
    };

    environment.systemPackages = [ pkgs.iproute2 ];

    systemd.services.d2b-test-agent-netns = {
      description = "Create the isolated network-agent test namespace";
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = pkgs.writeShellScript "d2b-test-agent-netns-up" ''
          set -eu
          install -d -m 0755 /run/netns
          ${pkgs.iproute2}/bin/ip netns add d2b-test-agent
          ${pkgs.iproute2}/bin/ip -n d2b-test-agent link set lo up
        '';
        ExecStop = "${pkgs.iproute2}/bin/ip netns delete d2b-test-agent";
      };
    };

    systemd.services.d2b-test-guest-agent = {
      description = "Network agent capability-confinement test process";
      requires = [ "d2b-test-agent-netns.service" ];
      after = [ "d2b-test-agent-netns.service" ];
      serviceConfig = {
        Type = "simple";
        User = "d2b-net-agent-test";
        Group = "d2b-net-agent-test";
        ExecStart = "${pkgs.coreutils}/bin/sleep infinity";
        NetworkNamespacePath = "/run/netns/d2b-test-agent";
        CapabilityBoundingSet = [
          "CAP_NET_ADMIN"
          "CAP_NET_BIND_SERVICE"
          "CAP_NET_RAW"
        ];
        AmbientCapabilities = [
          "CAP_NET_ADMIN"
          "CAP_NET_BIND_SERVICE"
          "CAP_NET_RAW"
        ];
        NoNewPrivileges = true;
      };
    };

    system.stateVersion = "25.11";
  };

  # The guest `guest-shell-service` boots: the component-session and
  # guest-broker modules applied directly to a NixOS node, with the enrolled
  # inputs the Guest target agent must boot from and the AF_VSOCK device its
  # ComponentSession listener binds.
  #
  # The check's fixture declared this node inline and carried its bundle and
  # key pair in its own `let`; both move here with it, so the guest survives
  # the fixture. The bundle is a fixture whose self-hash covers the canonical
  # JSON without `bundleHash`, and the key pair is the enrollment owner's
  # 32-byte inputs - the agent never generates either.
  d2bGuestShellServiceNode =
    { lib, pkgs, ... }:
    let
      fixtureKeys = pkgs.runCommand "guest-shell-component-session-keys" { } ''
        mkdir -p "$out"
        printf '\001\002\003\004\005\006\007\010\011\012\013\014\015\016\017\020\021\022\023\024\025\026\027\030\031\032\033\034\035\036\037\040' > "$out/guest.key"
        printf '\130\151\257\364\120\124\227\062\313\252\355\136\135\371\263\012\155\243\034\260\345\164\053\255\132\324\241\247\150\361\246\173' > "$out/parent.pub"
      '';

      guestBundle = pkgs.runCommand "guest-shell-guest-bundle" {
        nativeBuildInputs = [ pkgs.python3 ];
      } ''
        mkdir -p "$out"
        printf '%s\n' '{"schemaVersion":"v2","site":{"allowUnsafeEastWest":false},"environments":[],"nftables":{"family":"inet","table":"d2b","chains":[],"tableHashAfterApply":null,"ownershipId":"guest-shell-service"},"networkManager":{"filePath":"/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf","matchCriteria":[],"reloadBehavior":"atomic-reload","ownership":{"owner":"root","group":"root","mode":"0644","driftPolicy":"replace"}},"hostsFile":{"startMarker":"# d2b-managed begin","endMarker":"# d2b-managed end","rule":"replace-managed-block"},"kernelModules":[],"fdOwnership":[],"cloudHypervisorCapabilities":[],"ifNameMappings":[],"ch":null,"firewallCoexistencePolicy":null}' > "$out/host.json"
        printf '%s\n' '{"schemaVersion":"v2","vms":[]}' > "$out/processes.json"
        printf '%s\n' '{"schemaVersion":"v2","publicOperations":[],"brokerOperations":[]}' > "$out/privileges.json"
        printf '%s\n' '{"_manifest":{"manifestVersion":6},"_observability":{"enabled":false,"signozUrl":"http://127.0.0.1:8080","signozOtlpGrpcPort":4317,"signozOtlpHttpPort":4318,"obsVsockCid":0,"obsVsockHostSocket":"","vmName":""}}' > "$out/vms.json"
        python3 - "$out/bundle.json" <<'PY'
        import hashlib
        import json
        import sys

        # Zone-native v3 bundle: the loader (BundleResolver) accepts only the
        # v3 contract. The self-hash is computed over the serialization with
        # bundleHash absent and artifactHashes nullified (verify_bundle_hash).
        bundle = {
            "artifactHashes": {},
            "bundleVersion": 1,
            "schemaVersion": "v3",
            "privilegesPath": "privileges.json",
            "zones": [],
            "generation": {
                "generatedAt": None,
                "generator": "guest-shell-service",
                "sourceRevision": None,
            },
        }
        preimage = dict(bundle)
        preimage["artifactHashes"] = None
        canonical = json.dumps(preimage, sort_keys=True, separators=(",", ":")).encode()
        bundle["bundleHash"] = "sha256:" + hashlib.sha256(canonical).hexdigest()
        with open(sys.argv[1], "w", encoding="utf-8") as output:
            json.dump(bundle, output, sort_keys=True, separators=(",", ":"))
            output.write("\n")
        PY
      '';
    in
    { lib, pkgs, ... }:
    {
      imports = [
          ../../nixos-modules/component-session.nix
          ../../nixos-modules/guest-broker.nix
          {
            _module.args = {
              d2bInputs = { inherit self; };
              d2bHostTools = {
                broker = self.packages.${pkgs.system}.d2b-broker-guest-static;
              };
              d2bHostToolOverrides = self.lib.d2bHostToolOverrides;
            };

            d2b.componentSession = {
              enable = lib.mkForce true;
              guestConfigPath = lib.mkForce null;
            };

            # The Guest target agent binds an AF_VSOCK ComponentSession listener.
            # The lane's QEMU ships vhost-vsock-pci and now passes /dev/vhost-vsock
            # into the build sandbox, so this node can carry the same device the
            # enrolled Guest gets, plus a fixture bundle and key pair installed at
            # the production owner/mode the resolver verifies (root:d2bd 0640).
            virtualisation.qemu.options = [ "-device" "vhost-vsock-pci,guest-cid=3" ];
            boot.kernelModules = [ "vmw_vsock_virtio_transport" ];

            environment.etc."d2b/component-session/guest.key".source =
              "${fixtureKeys}/guest.key";
            environment.etc."d2b/component-session/parent.pub".source =
              "${fixtureKeys}/parent.pub";

            d2b.componentSession.localPrivateKeyPath =
              "/etc/d2b/component-session/guest.key";
            d2b.componentSession.parentPublicKeyPath =
              "/etc/d2b/component-session/parent.pub";
            d2b.componentSession.bundlePath = "/var/lib/d2b/guest-bundle/bundle.json";
            d2b.guestBroker.bundlePath = "/var/lib/d2b/guest-bundle/bundle.json";

            systemd.services.d2b-install-guest-bundle = {
              requiredBy = [ "d2bd-guest.service" "d2b-broker-guest.service" ];
              before = [ "d2bd-guest.service" "d2b-broker-guest.service" ];
              serviceConfig.Type = "oneshot";
              script = ''
                install -d -o root -g d2bd -m 0750 /var/lib/d2b/guest-bundle
                for file in bundle.json host.json processes.json privileges.json; do
                  install -o root -g d2bd -m 0640 \
                    ${guestBundle}/"$file" /var/lib/d2b/guest-bundle/"$file"
                done
                install -o root -g d2bd -m 0644 \
                  ${guestBundle}/vms.json /var/lib/d2b/guest-bundle/vms.json
              '';
            };

            system.stateVersion = "25.11";
          }
        ];
    };

  # The guest `wayland-proxy` boots: a plain NixOS node with the `alice` user
  # the proxy runs as, a Python interpreter for the fake compositor and the
  # client that drives it, and the proxy itself.
  #
  # The proxy is resolved through `self.packages`, which is how the fixture
  # resolved it: under the lane's handoff that package is the Bazel-built
  # host-tool bundle, so the guest runs the binary this build produces rather
  # than a second copy nix built.
  d2bWaylandProxyNode = { pkgs, ... }: {
    users.users.alice = {
      isNormalUser = true;
      uid = 1000;
    };

    environment.systemPackages = [
      pkgs.python3
      self.packages.${pkgs.stdenv.hostPlatform.system}.d2b-wayland-proxy
    ];

    system.stateVersion = "25.11";
  };

  # The guest `resource-operator-activation` boots: the reusable daemon node
  # plus the fixture's own contributions - nftables on, the acceptance
  # provider artifact and its publisher key, the two zones and their rows,
  # the `alice` and `bob` users, and `jq` for the CLI's answers.
  #
  # The `let` bindings the fixture carried move with the node, so the guest
  # survives the fixture exactly as the shape-only guests do.
  d2bResourceOperatorActivationNode =
    d2bDaemonNode {
      extra =
        { lib, pkgs, ... }:
        let
          d2bLib = import ../../tests/host-integration/lib.nix {
              inherit self;
            inherit lib;
            hostToolBundle =
              if self.lib ? d2bHostToolBundle then self.lib.d2bHostToolBundle else null;
          };
          providerArtifact = d2bLib.mkAcceptanceProviderArtifact pkgs;
          acceptancePublisherKey = providerArtifact.trustedPublisher.signingKey;
          artifacts = {
            acceptance-provider = {
              inherit (providerArtifact) package type catalog;
            };
          };
          hostRuntime = pkgs.writeText "d2b-acceptance-host-runtime.json" (builtins.toJSON {
            schemaVersion = "v1";
            bundleVersion = 1;
            generatedAt = "1970-01-01T00:00:00.000Z";
            nftAppliedHash = null;
            ifnames = [ ];
          });
        in
        {
          networking.nftables.enable = true;
          networking.nftables.ruleset = lib.mkAfter ''
            table inet d2b {}
          '';
          systemd.tmpfiles.rules = [
            "d /etc/NetworkManager/conf.d 0755 root root -"
          ];
          environment.etc."d2b/acceptance-host-runtime.json".source = hostRuntime;
          d2b.site.adminUsers = [ "alice" ];
          systemd.services.d2bd.serviceConfig.ExecStartPre = lib.mkAfter [
            "+${pkgs.writeShellScript "d2b-acceptance-hosts-prep" ''
              if [ -L /etc/hosts ]; then
                ${pkgs.coreutils}/bin/cat /etc/hosts > /run/d2b-acceptance-hosts
                ${pkgs.coreutils}/bin/rm -f /etc/hosts
                ${pkgs.coreutils}/bin/install -o root -g root -m 0644 \
                  /run/d2b-acceptance-hosts /etc/hosts
              fi
            ''}"
            "+${pkgs.writeShellScript "d2b-acceptance-host-runtime-prep" ''
              ${pkgs.coreutils}/bin/install -D -o root -g d2bd -m 0640 \
                /etc/d2b/acceptance-host-runtime.json \
                /var/lib/d2b/runtime/host-runtime.json
            ''}"
          ];
          users.users.bob = {
            isNormalUser = true;
            uid = 1001;
          };
          d2b.artifacts = artifacts;
          d2b.zones.local-root.trustedPublishers.d2b-u20-acceptance.signingKey =
            acceptancePublisherKey;
        d2b.zones.work.parentZone = "local-root";
        d2b.zones.work.trustedPublishers.d2b-u20-acceptance.signingKey =
          acceptancePublisherKey;
        d2b.zones.work.resources = {
          alice = {
            type = "User";
            spec = {
              displayName = "Alice";
              groups = [ ];
              osUsername = "alice";
            };
          };
          d2bd = {
            type = "User";
            spec = {
              displayName = "d2bd";
              groups = [ ];
              osUsername = "d2bd";
            };
          };
          operator-reader = {
            type = "Role";
            spec.rules = [
              {
                resourceTypes = [
                  "Host"
                  "Process"
                  "Provider"
                  "User"
                ];
                verbs = [ "get" "list" ];
                subresources = [ ];
                resourceNames = [ ];
                zones = [ "work" ];
                executionRefs = [ ];
                sessionVerbs = [ "connect" "invoke" ];
              }
            ];
          };
          operator-reader-binding = {
            type = "RoleBinding";
            spec = {
              roleRef = "Role/operator-reader";
              subjects = [ "User/alice" ];
              externalPrincipalSelector = null;
              scopeNarrowing = null;
            };
          };
          host-system = {
              type = "Host";
              spec = {
                providerRef = "Provider/system-core";
                defaultDomain = "system";
                allowedDomains = [ "system" ];
              budget = { };
              networkAttachments = [ ];
              deviceAttachments = [ ];
              volumeAttachmentDefaults = [ ];
            };
          };
            network-local = {
              type = "Provider";
              spec = {
                artifactId = "acceptance-provider";
                config.controllerExecutionRef = "Host/host-system";
              };
            };
          };
          environment.systemPackages = [ pkgs.jq ];
        };
    };

  # The guest `state-posture-contract` boots: the writable-store shape plus
  # the acceptance artifacts and zones, the guest system whose state chain is
  # checked, and the six userspace tools the assertions drive it with.
  #
  # The fixture's own `let` bindings come with it - the two provider
  # artifacts, the ComponentSession key pair, the v3 guest bundle, the Cloud
  # Hypervisor configuration, the checked guest system, its store-view image
  # and the installed artifacts - so the guest survives the fixture exactly as
  # the other ported guests do.
  d2bStatePostureContractNode =
    d2bCloudHypervisorNode {
      extra =
        { lib, pkgs, ... }:
        let
          d2bLib = import ../../tests/host-integration/lib.nix {
            inherit self;
            inherit lib;
            hostToolBundle =
              if self.lib ? d2bHostToolBundle then self.lib.d2bHostToolBundle else null;
          };
      cloudHypervisorArtifact =
        d2bLib.mkRuntimeCloudHypervisorArtifact pkgs;
      volumeProviderArtifact = d2bLib.mkVolumeProviderArtifact pkgs;
      fixtureKeys = pkgs.runCommand "acceptance-component-session-keys" { } ''
        mkdir -p "$out"
        printf '\001\002\003\004\005\006\007\010\011\012\013\014\015\016\017\020\021\022\023\024\025\026\027\030\031\032\033\034\035\036\037\040' > "$out/host.key"
        printf '\007\243\174\274\024\040\223\310\267\125\334\033\020\350\154\264\046\067\112\321\152\250\123\355\013\337\300\262\270\155\034\174' > "$out/host.pub"
        printf '\041\042\043\044\045\046\047\050\051\052\053\054\055\056\057\060\061\062\063\064\065\066\067\070\071\072\073\074\075\076\077\100' > "$out/guest.key"
        printf '\130\151\257\364\120\124\227\062\313\252\355\136\135\371\263\012\155\243\034\260\345\164\053\255\132\324\241\247\150\361\246\173' > "$out/guest.pub"
      '';
      guestBundle = pkgs.runCommand "acceptance-guest-bundle" {
        nativeBuildInputs = [ pkgs.python3 ];
      } ''
        mkdir -p "$out"
        cat > "$out/host.json" <<'EOF'
        {"schemaVersion":"v2","site":{"allowUnsafeEastWest":false},"environments":[],"nftables":{"family":"inet","table":"d2b","chains":[],"tableHashAfterApply":null,"ownershipId":"host-integration"},"networkManager":{"filePath":"/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf","matchCriteria":[],"reloadBehavior":"atomic-reload","ownership":{"owner":"root","group":"root","mode":"0644","driftPolicy":"replace"}},"hostsFile":{"startMarker":"# d2b-managed begin","endMarker":"# d2b-managed end","rule":"replace-managed-block"},"kernelModules":[],"fdOwnership":[],"cloudHypervisorCapabilities":[],"ifNameMappings":[],"ch":null,"firewallCoexistencePolicy":null}
        EOF
        printf '%s\n' '{"schemaVersion":"v2","vms":[]}' > "$out/processes.json"
        printf '%s\n' '{"schemaVersion":"v2","publicOperations":[],"brokerOperations":[]}' > "$out/privileges.json"
        printf '%s\n' '{"_manifest":{"manifestVersion":6},"_observability":{"enabled":false,"signozUrl":"http://127.0.0.1:8080","signozOtlpGrpcPort":4317,"signozOtlpHttpPort":4318,"obsVsockCid":0,"obsVsockHostSocket":"","vmName":""}}' > "$out/vms.json"
        python3 - "$out/bundle.json" <<'PY'
        import hashlib
        import json
        import sys

        # Zone-native v3 bundle: the loader (BundleResolver) accepts only the
        # v3 contract. The self-hash is computed over the serialization with
        # bundleHash absent and artifactHashes nullified (verify_bundle_hash).
        bundle = {
            "artifactHashes": {},
            "bundleVersion": 1,
            "schemaVersion": "v3",
            "privilegesPath": "privileges.json",
            "zones": [],
            "generation": {
                "generatedAt": None,
                "generator": "host-integration",
                "sourceRevision": None,
            },
        }
        preimage = dict(bundle)
        preimage["artifactHashes"] = None
        canonical = json.dumps(preimage, sort_keys=True, separators=(",", ":")).encode()
        bundle["bundleHash"] = "sha256:" + hashlib.sha256(canonical).hexdigest()
        with open(sys.argv[1], "w", encoding="utf-8") as output:
            json.dump(bundle, output, sort_keys=True, separators=(",", ":"))
            output.write("\n")
        PY
      '';

      cloudHypervisorConfig = {
        controllerExecutionRef = "Host/host-system";
        defaultVcpus = 2;
        defaultMemoryMb = 512;
        defaultMachineType = "microvm";
        watchdog = true;
        adoptionWindowMs = 30000;
        healthCheckIntervalMs = 5000;
        healthCheckTimeoutMs = 1000;
        healthCheckFailureThreshold = 3;
        startupDeadlineMs = 120000;
      };
      guestSystem = d2bLib.mkGuestSystem {
        inherit pkgs;
        name = "acceptance-guest";
        modules = [
          ({ lib, name, ... }: {
            boot.kernelParams = [ "console=ttyS0" "loglevel=7" ];
            environment.etc."d2b/component-session/guest.key".source =
              "${fixtureKeys}/guest.key";
            environment.etc."d2b/component-session/parent.pub".source =
              "${fixtureKeys}/host.pub";
            systemd.services.d2bd-guest = {
              environment = {
                RUST_LOG = "d2bd=debug";
              };
              serviceConfig = {
                ReadOnlyPaths = [
                  "/etc/d2b/component-session/guest.key"
                  "/etc/d2b/component-session/parent.pub"
                ];
                StandardOutput = lib.mkForce "journal+console";
                StandardError = lib.mkForce "journal+console";
              };
            };
            systemd.services.d2b-test-boot-identity = {
              wantedBy = [ "basic.target" ];
              before = [ "d2bd-guest.service" ];
              serviceConfig.Type = "oneshot";
              script = ''
                printf 'D2B_GUEST_BOOT_ID=%s\n' \
                  "$(${pkgs.coreutils}/bin/cat /proc/sys/kernel/random/boot_id)" \
                  > /dev/console
              '';
            };
            d2b.componentSession.localPrivateKeyPath =
              "/etc/d2b/component-session/guest.key";
            d2b.componentSession.parentPublicKeyPath =
              "/etc/d2b/component-session/parent.pub";
            d2b.componentSession.bundlePath =
              "/var/lib/d2b/guest-bundle/bundle.json";
            d2b.guestBroker.bundlePath =
              "/var/lib/d2b/guest-bundle/bundle.json";
            systemd.services.d2b-install-guest-bundle = {
              requiredBy = [ "d2b-broker-guest.service" "d2bd-guest.service" ];
              before = [ "d2b-broker-guest.service" "d2bd-guest.service" ];
              serviceConfig.Type = "oneshot";
              script = ''
                install -d -o root -g d2bd -m 0750 /var/lib/d2b/guest-bundle
                for file in bundle.json host.json processes.json privileges.json; do
                  install -o root -g d2bd -m 0640 \
                    ${guestBundle}/"$file" /var/lib/d2b/guest-bundle/"$file"
                done
                install -o root -g d2bd -m 0644 \
                  ${guestBundle}/vms.json /var/lib/d2b/guest-bundle/vms.json
              '';
            };
            networking.useDHCP = lib.mkForce false;
            networking.networkmanager.enable = lib.mkForce false;
            systemd.network.enable = lib.mkForce false;
            services.dbus.enable = lib.mkForce false;
            services.resolved.enable = lib.mkForce false;
            systemd.services.systemd-vconsole-setup.enable = false;
            d2b.vms.${name}.runner = {
              store.onDisk = true;
              store.disk = guestStoreDisk;
              shares = lib.mkForce [ ];
            };
            fileSystems."/nix/store" = {
              device = "/dev/vda";
              fsType = "ext4";
              options = [ "ro" "x-initrd.mount" ];
              neededForBoot = true;
            };
          })
        ];
      };
      guestClosure = pkgs.closureInfo {
        rootPaths = [ guestSystem.config.system.build.toplevel ];
      };
      guestStoreDisk = pkgs.runCommand "acceptance-guest-store.img" {
        nativeBuildInputs = [ pkgs.coreutils pkgs.e2fsprogs ];
      } ''
        mkdir -p root
        while IFS= read -r path; do
          cp -r --no-preserve=ownership,xattr,context "$path" root/
        done < ${guestClosure}/store-paths
        truncate -s 4096M "$out"
        # Reproducible ext4 image: SOURCE_DATE_EPOCH pins the superblock times and
        # a fixed UUID seed pins the htree hash seed (e2fsprogs ignores an all-zero
        # seed and randomizes it), so every build is byte-identical. With a random
        # seed each build differed, and the nixos-install closure spec (recorded
        # from an earlier build) could never match the freshly built image.
        SOURCE_DATE_EPOCH=0 mkfs.ext4 -q -F \
          -U 123e4567-e89b-12d3-a456-426614174000 \
          -E hash_seed=123e4567-e89b-12d3-a456-426614174000 \
          -d root "$out"
      '';
      artifacts = {
        runtime-cloud-hypervisor = {
          inherit (cloudHypervisorArtifact) package type catalog;
        };
        volume-acceptance-provider = {
          inherit (volumeProviderArtifact) package type catalog;
        };
        acceptance-system = {
          package = guestSystem.config.system.build.toplevel;
          type = "nixos-system";
        };
      };
        in
        {
        d2b.site.adminUsers = [ "alice" ];
        environment.systemPackages = with pkgs; [
          acl
          iproute2
          jq
          iputils
          procps
          util-linux
        ];
        d2b.artifacts = artifacts;
        # The declaration under test, installed verbatim from the repo so the
        # fixture reads the same file the posture code embeds.
        environment.etc."d2b/state-posture-contract.json".source =
          ../../packages/d2b-broker/src/ops/state-posture-contract.json;
        d2b.guestSystems.work.acceptance-guest = guestSystem;
        d2b.zones.local-root.trustedPublishers.d2b-cloud-hypervisor.signingKey =
          cloudHypervisorArtifact.trustedPublisher.signingKey;
        d2b.zones.local-root.trustedPublishers.d2b-volume-acceptance.signingKey =
          volumeProviderArtifact.trustedPublisher.signingKey;
        d2b.zones.work.trustedPublishers.d2b-cloud-hypervisor.signingKey =
          cloudHypervisorArtifact.trustedPublisher.signingKey;
        d2b.zones.work.trustedPublishers.d2b-volume-acceptance.signingKey =
          volumeProviderArtifact.trustedPublisher.signingKey;
        d2b.zones.local-root.resources.host-system = {
          type = "Host";
          spec = {
            providerRef = "Provider/system-core";
            defaultDomain = "system";
            allowedDomains = [ "system" ];
            budget = { };
            networkAttachments = [ ];
            deviceAttachments = [ ];
            volumeAttachmentDefaults = [ ];
          };
        };
        d2b.zones.work = {
          parentZone = "local-root";
          resources = {
            alice = {
              type = "User";
              spec = {
                displayName = "Alice";
                groups = [ ];
                osUsername = "alice";
              };
            };
            d2bd = {
              type = "User";
              spec = {
                displayName = "d2bd";
                groups = [ ];
                osUsername = "d2bd";
              };
            };
            lifecycle-operator = {
              type = "Role";
              spec.rules = [
                {
                  resourceTypes = [ "Endpoint" "Guest" "Host" "Process" "Provider" "Volume" "VolumeBinding" ];
                  verbs = [ "get" "list" ];
                  subresources = [ ];
                  resourceNames = [ ];
                  zones = [ "work" ];
                  executionRefs = [ ];
                  sessionVerbs = [ "connect" "invoke" ];
                }
                {
                  resourceTypes = [ "Guest" ];
                  verbs = [ "delete" ];
                  subresources = [ ];
                  resourceNames = [ "acceptance-guest" ];
                  zones = [ "work" ];
                  executionRefs = [ ];
                  sessionVerbs = [ "connect" "invoke" ];
                }
                {
                  resourceTypes = [ "Volume" ];
                  verbs = [ "delete" ];
                  subresources = [ ];
                  resourceNames = [ "state" ];
                  zones = [ "work" ];
                  executionRefs = [ ];
                  sessionVerbs = [ "connect" "invoke" ];
                }
              ];
            };
            lifecycle-operator-binding = {
              type = "RoleBinding";
              spec = {
                roleRef = "Role/lifecycle-operator";
                subjects = [ "User/alice" ];
                externalPrincipalSelector = null;
                scopeNarrowing = null;
              };
            };
            host-system = {
              type = "Host";
              spec = {
                providerRef = "Provider/system-core";
                defaultDomain = "system";
                allowedDomains = [ "system" ];
                budget = { };
                networkAttachments = [ ];
                deviceAttachments = [ ];
                volumeAttachmentDefaults = [ ];
              };
            };
            volume-local = {
              type = "Provider";
              spec = {
                artifactId = "volume-acceptance-provider";
                config = {
                  controllerExecutionRef = "Host/host-system";
                  sourcePolicies = [
                    {
                      id = "default-state";
                      class = "local-path";
                      volumeKinds = [ "durable" "state" "cache" ];
                    }
                    # U7: daemon-owned root the unprivileged daemon can
                    # lock and provision inline (path:daemon-state).
                    {
                      id = "daemon-state";
                      class = "local-path";
                      volumeKinds = [ "durable" "state" "cache" ];
                    }
                  ];
                };
              };
            };
            volume-virtiofs = {
              type = "Provider";
              spec = {
                artifactId = "volume-acceptance-provider";
                config.controllerExecutionRef = "Host/host-system";
              };
            };
            state = {
              type = "Volume";
              spec = {
                providerRef = "Provider/volume-local";
                kind = "state";
                source = {
                  executionRef = "Host/host-system";
                  settings = {
                    kind = "local-path";
                    sourcePolicyId = "daemon-state";
                  };
                };
                layout = [{
                  path = "state";
                  type = "directory";
                  # U7: daemon-owned so the unprivileged daemon can
                  # provision inline; the guest share stays read-only.
                  ownerRef = "User/d2bd";
                  groupRef = "User/d2bd";
                  mode = "0700";
                  target = null;
                  accessAcl = [ ];
                  defaultAcl = [ ];
                  foreignChildPolicy = "preserve";
                  noFollow = true;
                  recursive = false;
                  sensitivity = "private";
                  createPolicy = "create-if-never-provisioned";
                  repairPolicy = "exact-owner";
                  cleanupPolicy = "owner-controlled";
                  adoptionPolicy = "quarantine-on-ambiguity";
                  restartPolicy = "preserve-across-controller-restart";
                  leaseClass = "none";
                  invariants = [ "no-symlink" ];
                }];
                views.controller = {
                  path = "";
                  rights = [ "read" "write" "traverse" ];
                };
                # KTD1: the attachment stays declared input only. The Volume
                # side mints the durable VolumeBinding at reconcile; the
                # deterministic binding identity below is
                # vol-binding-6a8ea4307a30f7ceae6533f2 (volume, execution
                # target, view, mount path).
                attachments = [{
                  executionRef = "Guest/acceptance-guest";
                  transport = "virtiofs";
                  view = "controller";
                  access = "read-only";
                  mountPath = "/state";
                  settings = {
                    posixAcl = false;
                    xattr = false;
                    cache = "auto";
                    inodeFileHandles = "never";
                    threadPoolSize = null;
                    socketGroup = null;
                  };
                }];
              };
            };
            runtime-cloud-hypervisor = {
              type = "Provider";
              spec = {
                artifactId = "runtime-cloud-hypervisor";
                config = cloudHypervisorConfig;
              };
            };
            acceptance-guest = {
              type = "Guest";
              spec = {
                providerRef = "Provider/runtime-cloud-hypervisor";
                executionRef = "Host/host-system";
                systemArtifactId = "acceptance-system";
                defaultDomain = "system";
                allowedDomains = [ "system" ];
                budget = { };
                volumeAttachmentDefaults = [ ];
                networkAttachments = [ ];
                deviceAttachments = [ ];
              };
            };
          };
        };
      };
    };

  # The guest `virtiofsd-volume-runtime` boots: the reusable daemon node
  # plus the fixture's own contributions.
  #
  # The fixture declared no machine size, disk or device of its own - the shape
  # carries those - so what moves here is its `let` bindings (the Volume
  # acceptance artifact and the acceptance host runtime) and the extra module
  # that turns nftables on, installs the host runtime, declares the two zones
  # and their rows, and adds `jq` and `procps`.
  d2bVirtiofsdVolumeRuntimeNode =
    d2bDaemonNode {
      extra =
        { lib, pkgs, ... }:
        let
      d2bLib = import ../../tests/host-integration/lib.nix {
        inherit self;
        inherit lib;
        hostToolBundle =
          if self.lib ? d2bHostToolBundle then self.lib.d2bHostToolBundle else null;
      };
      volumeProviderArtifact = d2bLib.mkVolumeProviderArtifact pkgs;
      artifacts = {
        volume-acceptance-provider = {
          inherit (volumeProviderArtifact) package type catalog;
        };
      };
      hostRuntime = pkgs.writeText "d2b-acceptance-host-runtime.json" (builtins.toJSON {
        schemaVersion = "v1";
        bundleVersion = 1;
        generatedAt = "1970-01-01T00:00:00.000Z";
        nftAppliedHash = null;
        ifnames = [ ];
      });
        in
        {
          networking.nftables.enable = true;
          networking.nftables.ruleset = lib.mkAfter ''
            table inet d2b {}
          '';
          systemd.tmpfiles.rules = [
            "d /etc/NetworkManager/conf.d 0755 root root -"
          ];
          environment.etc."d2b/acceptance-host-runtime.json".source = hostRuntime;
          d2b.site.adminUsers = [ "alice" ];
          systemd.services.d2bd.serviceConfig.ExecStartPre = lib.mkAfter [
            "+${pkgs.writeShellScript "d2b-acceptance-hosts-prep" ''
              if [ -L /etc/hosts ]; then
                ${pkgs.coreutils}/bin/cat /etc/hosts > /run/d2b-acceptance-hosts
                ${pkgs.coreutils}/bin/rm -f /etc/hosts
                ${pkgs.coreutils}/bin/install -o root -g root -m 0644 \
                  /run/d2b-acceptance-hosts /etc/hosts
              fi
            ''}"
            "+${pkgs.writeShellScript "d2b-acceptance-host-runtime-prep" ''
              ${pkgs.coreutils}/bin/install -D -o root -g d2bd -m 0640 \
                /etc/d2b/acceptance-host-runtime.json \
                /var/lib/d2b/runtime/host-runtime.json
            ''}"
          ];
          d2b.artifacts = artifacts;
          d2b.zones.local-root.trustedPublishers.d2b-volume-acceptance.signingKey =
            volumeProviderArtifact.trustedPublisher.signingKey;
        d2b.zones.work.parentZone = "local-root";
        d2b.zones.work.trustedPublishers.d2b-volume-acceptance.signingKey =
          volumeProviderArtifact.trustedPublisher.signingKey;
        d2b.zones.work.resources = {
          alice = {
            type = "User";
            spec = {
              displayName = "Alice";
              groups = [ ];
              osUsername = "alice";
            };
          };
          d2bd = {
            type = "User";
            spec = {
              displayName = "d2bd";
              groups = [ ];
              osUsername = "d2bd";
            };
          };
          volume-operator = {
            type = "Role";
            spec.rules = [
              {
                resourceTypes = [
                  "Endpoint"
                  "Host"
                  "Process"
                  "Provider"
                  "Volume"
                  "VolumeBinding"
                ];
                verbs = [ "get" "list" ];
                subresources = [ ];
                resourceNames = [ ];
                zones = [ "work" ];
                executionRefs = [ ];
                sessionVerbs = [ "connect" "invoke" ];
              }
              {
                resourceTypes = [ "Volume" ];
                verbs = [ "delete" ];
                subresources = [ ];
                resourceNames = [ "state" ];
                zones = [ "work" ];
                executionRefs = [ ];
                sessionVerbs = [ "connect" "invoke" ];
              }
            ];
          };
          volume-operator-binding = {
            type = "RoleBinding";
            spec = {
              roleRef = "Role/volume-operator";
              subjects = [ "User/alice" ];
              externalPrincipalSelector = null;
              scopeNarrowing = null;
            };
          };
          # The attachment execution target. The Nix bundle validation requires
          # attachment refs to resolve to a same-Zone Host or Guest; the
          # virtiofs serving path is host-side (the binding mints its socket
          # under /run/d2b/vms/<guest>/), so this fixture asserts the Volume
          # chain only - the Guest row itself stays declared input (KTD1).
          acceptance-guest = {
            type = "Guest";
            spec = {
              defaultDomain = "system";
              providerRef = "Provider/volume-virtiofs";
              budget = { };
              networkAttachments = [ ];
              deviceAttachments = [ ];
              volumeAttachmentDefaults = [ ];
            };
          };
          host-system = {
            type = "Host";
            spec = {
              providerRef = "Provider/system-core";
              defaultDomain = "system";
              allowedDomains = [ "system" ];
              budget = { };
              networkAttachments = [ ];
              deviceAttachments = [ ];
              volumeAttachmentDefaults = [ ];
            };
          };
          # The fixed daemon-owned Volume owner: volume-local is the only
          # Provider a Volume may select (U7 driver contract).
          volume-local = {
            type = "Provider";
            spec = {
              artifactId = "volume-acceptance-provider";
              config = {
                controllerExecutionRef = "Host/host-system";
                sourcePolicies = [
                  {
                    id = "daemon-state";
                    class = "local-path";
                    volumeKinds = [ "durable" "state" "cache" ];
                  }
                ];
              };
            };
          };
          # The serving Provider the derived VolumeBinding rows select and
          # whose signed virtiofsd-worker template the worker launch resolves.
          volume-virtiofs = {
            type = "Provider";
            spec = {
              artifactId = "volume-acceptance-provider";
              config.controllerExecutionRef = "Host/host-system";
            };
          };
          state = {
            type = "Volume";
            spec = {
              providerRef = "Provider/volume-local";
              kind = "state";
              source = {
                executionRef = "Host/host-system";
                settings = {
                  kind = "local-path";
                  sourcePolicyId = "daemon-state";
                };
              };
              layout = [{
                path = "state";
                type = "directory";
                # Daemon-owned so the unprivileged daemon can provision the
                # local-path layout inline.
                ownerRef = "User/d2bd";
                groupRef = "User/d2bd";
                mode = "0700";
                target = null;
                accessAcl = [ ];
                defaultAcl = [ ];
                foreignChildPolicy = "preserve";
                noFollow = true;
                recursive = false;
                sensitivity = "private";
                createPolicy = "create-if-never-provisioned";
                repairPolicy = "exact-owner";
                cleanupPolicy = "owner-controlled";
                adoptionPolicy = "quarantine-on-ambiguity";
                restartPolicy = "preserve-across-controller-restart";
                leaseClass = "none";
                invariants = [ "no-symlink" ];
              }];
              views.controller = {
                path = "";
                rights = [ "read" "write" "traverse" ];
              };
              # KTD1: the attachment stays declared input only. The Volume side
              # mints the durable VolumeBinding at reconcile; the deterministic
              # binding identity is derived from (volume, execution target,
              # view, mount path) - the fixture asserts that exact identity.
              attachments = [{
                executionRef = "Guest/acceptance-guest";
                transport = "virtiofs";
                view = "controller";
                access = "read-only";
                mountPath = "/state";
                settings = {
                  posixAcl = false;
                  xattr = false;
                  cache = "auto";
                  inodeFileHandles = "never";
                  threadPoolSize = null;
                  socketGroup = null;
                };
              }];
            };
          };
        };
        environment.systemPackages = with pkgs; [
          jq
          procps
        ];
    };
  };

  # The guest `device-worker-launch` boots: the reusable daemon node
  # plus the fixture's own contributions.
  #
  # The fixture declared no machine size, disk or device of its own - the shape
  # carries those - so what moves here is its `let` bindings (the swtpm and GPU
  # device-worker artifacts, the crosvm stand-in shim, the Cloud Hypervisor
  # configuration and the declared artifacts) and the extra module that declares
  # the provider rows and the Devices whose worker rows the check launches.
  d2bDeviceWorkerLaunchNode =
    d2bDaemonNode {
      extra =
        { lib, pkgs, ... }:
        let
      hostToolBundle =
        if self.lib ? d2bHostToolBundle then self.lib.d2bHostToolBundle else null;
      d2bLib = import ../../tests/host-integration/lib.nix {
        inherit self;
        inherit lib;
        inherit hostToolBundle;
      };
      cloudHypervisorArtifact = d2bLib.mkRuntimeCloudHypervisorArtifact pkgs;
      volumeProviderArtifact = d2bLib.mkVolumeProviderArtifact pkgs;

      # A Provider artifact that packages the Device worker executables the
      # declared rows name. Shape mirrors `mkVolumeProviderArtifact`: a signed
      # manifest whose executable set is computed from the packaged `bin/` files,
      # a Device-exporting catalog entry, and a deterministic publisher key.
      mkDeviceWorkerProviderArtifact =
        { artifactId
        , publisher
        , binaries
        , controllerBinary
        }:
        let
          signer = pkgs.python3.withPackages
            (pythonPackages: [ pythonPackages.cryptography ]);
          manifest = ../../tests/fixtures/provider-acceptance/provider-manifest.json;
          schema = ../../tests/fixtures/provider-acceptance/config-schema.json;
          controller = if hostToolBundle == null then
            "${self.packages.${pkgs.stdenv.hostPlatform.system}.d2b-provider-test-controller}/bin/d2b-provider-test-controller"
          else
            "${hostToolBundle}/bin/d2b-provider-test-controller";
          package = pkgs.runCommand "d2b-${artifactId}" {
            nativeBuildInputs = [ pkgs.coreutils signer ];
          } ''
            mkdir -p "$out/bin"
            ${lib.concatStringsSep "\n" (lib.mapAttrsToList
              (name: path: ''
                cp "${path}" "$out/bin/${name}"
                chmod 0755 "$out/bin/${name}"
              '')
              binaries)}
            cp "${controller}" "$out/bin/${controllerBinary}"
            chmod 0755 "$out/bin/${controllerBinary}"
            ${signer}/bin/python3 - "${manifest}" "$out" \
              "${artifactId}" "${publisher}" "${controllerBinary}" \
              ${lib.escapeShellArg (lib.concatStringsSep " " (lib.attrNames binaries))} <<'PY'
            import hashlib
            import json
            import pathlib
            import sys
            from cryptography.hazmat.primitives import serialization
            from cryptography.hazmat.primitives.asymmetric.ed25519 import (
                Ed25519PrivateKey,
            )

            (
                manifest_path,
                output_path,
                artifact_id,
                publisher,
                controller_binary,
                binary_names,
            ) = sys.argv[1:]
            output = pathlib.Path(output_path)
            manifest = json.loads(pathlib.Path(manifest_path).read_text())
            # The executable set the compiler recomputes covers every regular file
            # in bin/: the controller binary plus the declared worker binaries.
            names = sorted(set(binary_names.split()) | {controller_binary})

            # Device-only manifest: the declared Device worker rows are the only
            # rows this artifact's Provider serves in this fixture.
            resource_types = {"Device"}
            manifest["apiBindings"] = [
                binding
                for binding in manifest.get("apiBindings", [])
                if binding.get("resourceType") in resource_types
            ]
            for component in manifest.get("components", []):
                component["exportedResourceTypes"] = [
                    resource_type
                    for resource_type in component.get("exportedResourceTypes", [])
                    if resource_type in resource_types
                ]

            manifest["artifactId"] = artifact_id
            manifest["trust"]["publisher"] = publisher
            executable_map = json.dumps(
                {
                    name: "sha256:" + hashlib.sha256(
                        (output / "bin" / name).read_bytes()
                    ).hexdigest()
                    for name in names
                },
                ensure_ascii=False,
                sort_keys=True,
                separators=(",", ":"),
            ).encode()
            first = hashlib.sha256(
                b"d2b:v3:provider-executable-set\0" + executable_map
            ).digest()
            executable_digest = "sha256:" + hashlib.sha256(first).hexdigest()
            controller_digest = "sha256:" + hashlib.sha256(
                (output / "bin" / controller_binary).read_bytes()
            ).hexdigest()
            manifest["digests"]["executable"] = executable_digest
            for component in manifest.get("components", []):
                for capability in component.get("targetCapabilities", []):
                    capability["artifactDigest"] = controller_digest
            manifest_bytes = json.dumps(
                manifest,
                ensure_ascii=False,
                sort_keys=True,
                separators=(",", ":"),
            ).encode()
            seed = hashlib.sha256(
                b"d2b-u17-device-worker-provider-signing-key-v1"
                + artifact_id.encode()
            ).digest()
            private_key = Ed25519PrivateKey.from_private_bytes(seed)
            public_key = private_key.public_key().public_bytes(
                serialization.Encoding.PEM,
                serialization.PublicFormat.SubjectPublicKeyInfo,
            )
            metadata = output / "share/d2b/provider"
            metadata.mkdir(parents=True)
            (metadata / "provider-manifest.json").write_bytes(manifest_bytes)
            (metadata / "provider-manifest.json.sig").write_bytes(
                private_key.sign(manifest_bytes)
            )
            (metadata / "config-schema.json").write_bytes(
                pathlib.Path("${schema}").read_bytes()
            )
            (output / "publisher-public-key.pem").write_bytes(public_key)
            (output / "executable-set-digest").write_text(executable_digest)
            (output / "manifest-digest").write_text(
                "sha256:" + hashlib.sha256(manifest_bytes).hexdigest()
            )
            PY
          '';
          packageDigestPath = pkgs.runCommand
            "d2b-${artifactId}-nar-digest" {
              nativeBuildInputs = [ pkgs.nix ];
            } ''
              printf 'sha256:%s' \
                "$(${pkgs.nix}/bin/nix --extra-experimental-features nix-command \
                  hash path --type sha256 --base16 "${package}")" > "$out"
            '';
          baseManifest = builtins.fromJSON (builtins.readFile manifest);
          catalog = {
            providerName = artifactId;
            packageName = "d2b-${artifactId}";
            version = "0.0.0";
            systems = [ pkgs.stdenv.hostPlatform.system ];
            platform = pkgs.stdenv.hostPlatform.system;
            apiCompatibility = "d2b.zone.v3";
            serviceCompatibility = "d2bd.resource";
            signature = { signatureId = "default"; };
            rootEpoch = 1;
            revocationStatus = "clear";
            denyStatus = "clear";
            provenanceEvidence = "accepted";
            sbomEvidence = "accepted";
            licenseEvidence = "accepted";
            vulnerabilityEvidence = "accepted";
            conformanceAttestation = "accepted";
            supportChannel = "stable";
            supportContact = "d2b-u17-device-worker@localhost";
            publisher = publisher;
            packageDigest = lib.removeSuffix "\n"
              (builtins.readFile packageDigestPath);
            executableDigest = lib.removeSuffix "\n"
              (builtins.readFile "${package}/executable-set-digest");
            manifestDigest = lib.removeSuffix "\n"
              (builtins.readFile "${package}/manifest-digest");
            componentDigest = "sha256:${builtins.hashString
              "sha256" (builtins.toJSON baseManifest.components)}";
            descriptorDigest = "sha256:${builtins.hashString
              "sha256" (builtins.toJSON baseManifest.apiBindings)}";
            configDigest = "sha256:${builtins.hashString
              "sha256" (builtins.readFile schema)}";
          };
        in {
          inherit package catalog;
          type = "provider";
          trustedPublisher = {
            publisherRef = publisher;
            signingKey = builtins.readFile "${package}/publisher-public-key.pem";
          };
        };

      # The GPU artifact's crosvm stand-in: a real ELF (via the shim) so the
      # artifact validates as a Provider executable set, whose behavior is to
      # record its argv and refuse. The fixture never asks a GPU worker to serve.
      crosvmStandIn = self.lib.buildProviderElfShim {
        inherit pkgs;
        name = "crosvm";
        interpreterPkg = pkgs.bash;
        interpreterPath = "bin/bash";
        program = pkgs.writeText "d2b-u17-crosvm-stand-in.sh" ''
          # Fixture stand-in for the GPU Provider's crosvm. It must never run in
          # a passing fixture (the launch path is proved up to the broker's own
          # refusal); if it does run, it records the argv the Process controller
          # composed and refuses, so a fabricated success is impossible.
          set -eu
          log="/run/d2b/u17-device-worker-standin.argv"
          if [ -d /run/d2b ]; then
            printf '%s\n' "crosvm-stand-in:$*" >> "$log" 2>/dev/null || true
          fi
          printf 'd2b-u17: crosvm stand-in invoked with %s\n' "$*" >&2
          exit 79
        '';
      };

      # One artifact serves both Device Providers: a Provider artifact exports its
      # ResourceTypes, and two artifacts both exporting `Device` collide in one
      # Zone (`provider-resourcetype-collision`).
      deviceWorkerArtifact = mkDeviceWorkerProviderArtifact {
        artifactId = "device-worker-acceptance-provider";
        publisher = "d2b-u17-device-worker";
        controllerBinary = "acceptance-controller";
        binaries = {
          swtpm = "${pkgs.swtpm}/bin/swtpm";
          swtpm-ioctl = "${pkgs.swtpm}/bin/swtpm_ioctl";
          crosvm = "${crosvmStandIn}/bin/crosvm";
        };
      };

      cloudHypervisorConfig = {
        controllerExecutionRef = "Host/host-system";
        defaultVcpus = 2;
        defaultMemoryMb = 512;
        defaultMachineType = "microvm";
        watchdog = true;
        adoptionWindowMs = 30000;
        healthCheckIntervalMs = 5000;
        healthCheckTimeoutMs = 1000;
        healthCheckFailureThreshold = 3;
        startupDeadlineMs = 120000;
      };

      artifacts = {
        runtime-cloud-hypervisor = {
          inherit (cloudHypervisorArtifact) package type catalog;
        };
        volume-acceptance-provider = {
          inherit (volumeProviderArtifact) package type catalog;
        };
        device-worker-acceptance-provider = {
          inherit (deviceWorkerArtifact) package type catalog;
        };
      };
        in
        {
        d2b.site.adminUsers = [ "alice" ];
        environment.systemPackages = with pkgs; [
          jq
          procps
          util-linux
          acl
          iproute2
          # The fixture binds its stale video socket from the VM side; python3
          # is the smallest reliable AF_UNIX binder available in the VM.
          python3
        ];
        d2b.artifacts = artifacts;
        d2b.zones.local-root.resources.host-system = {
          type = "Host";
          spec = {
            providerRef = "Provider/system-core";
            defaultDomain = "system";
            allowedDomains = [ "system" ];
            budget = { };
            networkAttachments = [ ];
            deviceAttachments = [ ];
            volumeAttachmentDefaults = [ ];
          };
        };
        d2b.zones.work.parentZone = "local-root";
        # Every Zone the host compiles a bundle for declares the publishers of
        # the artifacts its rows select; `local-root` is a compiled Zone too
        # (the other Cloud Hypervisor fixtures declare the same pair there).
        d2b.zones.local-root.trustedPublishers.d2b-cloud-hypervisor.signingKey =
          cloudHypervisorArtifact.trustedPublisher.signingKey;
        d2b.zones.local-root.trustedPublishers.d2b-volume-acceptance.signingKey =
          volumeProviderArtifact.trustedPublisher.signingKey;
        d2b.zones.local-root.trustedPublishers.d2b-u17-device-worker.signingKey =
          deviceWorkerArtifact.trustedPublisher.signingKey;
        d2b.zones.work.trustedPublishers.d2b-cloud-hypervisor.signingKey =
          cloudHypervisorArtifact.trustedPublisher.signingKey;
        d2b.zones.work.trustedPublishers.d2b-volume-acceptance.signingKey =
          volumeProviderArtifact.trustedPublisher.signingKey;
        d2b.zones.work.trustedPublishers.d2b-u17-device-worker.signingKey =
          deviceWorkerArtifact.trustedPublisher.signingKey;
        d2b.zones.work.resources = {
          alice = {
            type = "User";
            spec = {
              displayName = "Alice";
              groups = [ ];
              osUsername = "alice";
            };
          };
          d2bd = {
            type = "User";
            spec = {
              displayName = "d2bd";
              groups = [ ];
              osUsername = "d2bd";
            };
          };
          device-operator = {
            type = "Role";
            spec.rules = [
              {
                resourceTypes = [
                  "Device"
                  "Endpoint"
                  "EphemeralProcess"
                  "Guest"
                  "Host"
                  "Process"
                  "Provider"
                  "Volume"
                ];
                verbs = [ "get" "list" ];
                subresources = [ ];
                resourceNames = [ ];
                zones = [ "work" ];
                executionRefs = [ ];
                sessionVerbs = [ "connect" "invoke" ];
              }
              {
                resourceTypes = [ "Device" ];
                verbs = [ "delete" ];
                subresources = [ ];
                resourceNames = [ "tpm0" ];
                zones = [ "work" ];
                executionRefs = [ ];
                sessionVerbs = [ "connect" "invoke" ];
              }
            ];
          };
          device-operator-binding = {
            type = "RoleBinding";
            spec = {
              roleRef = "Role/device-operator";
              subjects = [ "User/alice" ];
              externalPrincipalSelector = null;
              scopeNarrowing = null;
            };
          };
          host-system = {
            type = "Host";
            spec = {
              providerRef = "Provider/system-core";
              defaultDomain = "system";
              allowedDomains = [ "system" ];
              budget = { };
              networkAttachments = [ ];
              deviceAttachments = [ ];
              volumeAttachmentDefaults = [ ];
            };
          };
          # The Device owners. The Guest stays declared input and is never
          # booted: the Device worker rows are bundle-declared `Process` rows of
          # the Process controller, and no guest system artifact is declared, so
          # no VMM is ever launched here. Its name is the VM identity of the
          # Devices it owns (`Device.metadata.ownerRef`).
          acceptance-guest = {
            type = "Guest";
            spec = {
              providerRef = "Provider/volume-virtiofs";
              executionRef = "Host/host-system";
              defaultDomain = "system";
              allowedDomains = [ "system" ];
              budget = { };
              volumeAttachmentDefaults = [ ];
              networkAttachments = [ ];
              deviceAttachments = [ ];
            };
          };
          volume-local = {
            type = "Provider";
            spec = {
              artifactId = "volume-acceptance-provider";
              config = {
                controllerExecutionRef = "Host/host-system";
                sourcePolicies = [
                  {
                    id = "daemon-state";
                    class = "local-path";
                    volumeKinds = [ "durable" "state" "cache" ];
                  }
                  # The TPM state Volume's source policy
                  # (`build_tpm_state_volume_spec`, opaque policy id).
                  {
                    id = "tpm-state";
                    class = "local-path";
                    volumeKinds = [ "state" ];
                  }
                ];
              };
            };
          };
          volume-virtiofs = {
            type = "Provider";
            spec = {
              artifactId = "volume-acceptance-provider";
              config.controllerExecutionRef = "Host/host-system";
            };
          };
          runtime-cloud-hypervisor = {
            type = "Provider";
            spec = {
              artifactId = "runtime-cloud-hypervisor";
              config = cloudHypervisorConfig;
            };
          };
          device-tpm = {
            type = "Provider";
            spec = {
              artifactId = "device-worker-acceptance-provider";
              config.controllerExecutionRef = "Host/host-system";
            };
          };
          device-gpu = {
            type = "Provider";
            spec = {
              artifactId = "device-worker-acceptance-provider";
              config.controllerExecutionRef = "Host/host-system";
            };
          };
          # The Device under test: an emulated TPM claimed by the Guest. The
          # Provider's projection declares `Process/swtpm-tpm0`,
          # `EphemeralProcess/swtpm-flush-tpm0`, `Endpoint/tpm-tpm0` and
          # `Endpoint/tpm-ctrl-tpm0` as this Device's children.
          tpm0 = {
            type = "Device";
            metadata.ownerRef = "Guest/acceptance-guest";
            spec = {
              providerRef = "Provider/device-tpm";
              deviceClass = "emulated";
              arbitration = "exclusive";
              maxConcurrentClaims = 1;
              inventory.selector = { };
            };
          };
          # The GPU/video Devices: a full GPU with its video sidecar
          # (`gpu-worker` + `video-worker` rows) and a render-node-only Device
          # (`gpu-render-node` row, the shape whose render node the broker
          # pre-opens itself). Both are physical DRM Devices by declaration; the
          # VM has no GPU, which is exactly what the fixture measures.
          gpu0 = {
            type = "Device";
            metadata.ownerRef = "Guest/acceptance-guest";
            spec = {
              providerRef = "Provider/device-gpu";
              deviceClass = "physical";
              arbitration = "exclusive";
              maxConcurrentClaims = 1;
              inventory.selector = { busClass = "drm"; label = "u17-gpu0"; };
            };
          };
          gpu1 = {
            type = "Device";
            metadata.ownerRef = "Guest/acceptance-guest";
            spec = {
              providerRef = "Provider/device-gpu";
              deviceClass = "physical";
              arbitration = "exclusive";
              maxConcurrentClaims = 1;
              inventory.selector = { busClass = "drm"; label = "u17-gpu1"; };
            };
          };
        };
    };
  };

  # The guest each fixture-less image evaluates, by the name the image action
  # asks for. A check's own guest is read out of the check's fixture; these are
  # the guests with no fixture to be read out of - the two reusable shapes the
  # lane's own images boot, and one entry per check whose assertions have moved
  # to the lane's own Rust and whose fixture is therefore gone.
  #
  # Naming an image that is in neither table is an error rather than a silent
  # fallback to the default node: a check that boots a guest nobody declared is
  # a check asserting against something no declaration describes. The tables
  # are also what keeps the two shape images and the ported checks apart, so a
  # mistyped check name cannot quietly become a shape-only image the lane skips.
  #
  #   shapeNodes        the reusable shapes, by name.
  #   portedCheckNodes  a ported check's guest, by the check's own name - the
  #                     name the lane reports it under and the name its image
  #                     is built for. `testName` is the name its fixture was
  #                     booted under, kept because that is the alias a
  #                     contributor filtering the lane may have read.
  #
  # One entry reaches one guest. A check that ports adds its node to
  # `portedCheckNodes` and takes its fixture away; nothing else in the lane has
  # to learn the check's name.
  shapeNodes = {
    daemon = d2bDaemonNode { };
    writable-store = d2bCloudHypervisorNode { };
  };

  portedCheckNodes = {
    bridge-isolation = {
      node = d2bBridgeIsolationNode;
      testName = "d2b-bridge-isolation";
    };
    daemon-smoke = {
      node = d2bDaemonSmokeNode;
      testName = "d2b-daemon-smoke";
    };
    device-worker-launch = {
      node = d2bDeviceWorkerLaunchNode;
      testName = "d2b-device-worker-launch";
    };
    guest-agent-cap-confinement = {
      node = d2bGuestAgentCapConfinementNode;
      testName = "d2b-guest-agent-cap-confinement";
    };
    guest-shell-service = {
      node = d2bGuestShellServiceNode;
      testName = "d2b-guest-shell-service";
    };
    privilege-oracle = {
      node = d2bDaemonNode { };
      testName = "d2b-privilege-oracle";
    };
    resource-operator-activation = {
      node = d2bResourceOperatorActivationNode;
      testName = "d2b-resource-operator-activation";
    };
    state-posture-contract = {
      node = d2bStatePostureContractNode;
      testName = "d2b-state-posture-contract";
    };
    virtiofsd-volume-runtime = {
      node = d2bVirtiofsdVolumeRuntimeNode;
      testName = "d2b-virtiofsd-volume-runtime";
    };
    wayland-proxy = {
      node = d2bWaylandProxyNode;
      testName = "d2b-wayland-proxy";
    };
  };
}
