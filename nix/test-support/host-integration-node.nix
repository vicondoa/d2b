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
    guest-agent-cap-confinement = {
      node = d2bGuestAgentCapConfinementNode;
      testName = "d2b-guest-agent-cap-confinement";
    };
    guest-shell-service = {
      node = d2bGuestShellServiceNode;
      testName = "d2b-guest-shell-service";
    };

  };
}
