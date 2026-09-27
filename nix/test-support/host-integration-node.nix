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
    daemon-smoke = {
      node = d2bDaemonSmokeNode;
      testName = "d2b-daemon-smoke";
    };
  };
}
