# Guest image for the Bazel-owned host-integration lane.
#
# The lane's guest-image action (`bazel/checks/vm/defs.bzl`) calls this
# through the flake's `guestImage` output, passing the d2b host binaries it
# received as declared Bazel label inputs. The guest closure is therefore
# keyed on the Bazel graph rather than on a developer's shell: `rawBundle`
# and `rawCloudHypervisorController` are the staged binary directories, by
# the same contract the legacy `D2B_HOST_TOOL_BUNDLE` handoff passes, and
# the caller content-addresses them.
#
# Two guest shapes are declared here, and the shape is a declared input
# rather than a second copy of the numbers: `daemon` is the node the current
# `vmChecks` fixtures boot for the daemon/broker host checks, and
# `writable-store` is the node the two nested-guest checks boot, which
# replaces the root drive and boots through a bootloader. Per-check module
# contributions arrive through `extraModules`.
#
# The output is one store path holding the guest's system closure, the root
# disk in the shape's own format, and a manifest of what a launcher needs to
# boot it: the machine size, the drive layout, the boot method with its
# kernel command line resolved, the per-check device options, and the
# activation contract the launcher waits for on the guest's serial console.
# The manifest is the only record of the invocation shape: the launcher
# renders it and never restates a number the re-homed node declared.
{ pkgs, self, bazelHostTools, rawBundle, extraModules ? [ ], nodeShape ? "daemon" }:

let
  inherit (pkgs) lib;

  # The lane's activation contract, in the repository's own field names:
  # `nixos-modules/lib.nix` describes every service capability with a
  # readiness signal and a contract that signal buys. The lane's guest
  # declares the same two fields - `readiness = "console-marker"` and
  # `contract = "d2b-daemon-acceptance"` - and the launcher waits for the
  # marker the contract names, so "the guest activated" means here what it
  # means everywhere else in the tree.
  activationMarker = "D2B_LANE_READY";

  # The guest-side bound on that wait. It is the backstop behind the
  # launcher's own bounded wait, and is deliberately the longer of the two:
  # the launcher fails the lane with the guest's console attached, which is
  # a better failure than a unit that dies silently first.
  activationTimeoutSeconds = 1800;
  # How long a polled unit may sit not-active before the guest describes the
  # ordering state it is sitting in, and how long after that it repeats
  # itself. Both are well inside the launcher's own bound, which is the whole
  # point: a report written after the launcher has stopped reading the
  # console is not a report, and the launcher's bound is the shorter of the
  # two. The repeat is deliberately wide, because a second copy of an
  # unchanged ordering state is console noise rather than evidence.
  activationStallSeconds = 90;
  activationStallRepeatSeconds = 600;

  # Refuse an incomplete handoff before a guest closure is evaluated. The
  # host-tool package repeats this check when it is built, but that is the
  # wrong place to learn a binary is missing: by then the guest closure has
  # been realized.
  stagedEntries = builtins.filter (name: name != "." && name != "..")
    (lib.attrNames (builtins.readDir (/. + rawBundle)));
  missing = lib.filter (name: !(builtins.elem name stagedEntries)) bazelHostTools.inventory;
  unexpected = lib.filter (name: !(builtins.elem name bazelHostTools.inventory)) stagedEntries;

  # The shared node configuration, re-homed so the guest outlives the
  # fixtures that used to declare it. The Bazel host-tool package reaches
  # the guest through the self override, not through this module.
  d2bNode = import ./host-integration-node.nix {
    self = self;
    inherit (pkgs) lib;
  };

  # `d2bDaemonNode` declares `virtualisation.*`, so the guest is evaluated
  # with the same QEMU VM module the runNixOSTest nodes carry. Evaluating
  # the node module directly, rather than through the test driver, is what
  # makes the result a bootable system closure the lane's own launcher can
  # use.
  evaluated = import (pkgs.path + "/nixos/lib/eval-config.nix") {
    system = pkgs.stdenv.hostPlatform.system;
    modules = [
      (pkgs.path + "/nixos/modules/virtualisation/qemu-vm.nix")
      # `d2bCloudHypervisorNode` is `d2bDaemonNode` with the writable store
      # opted into, so the two shapes are one declaration read two ways and
      # cannot drift apart.
      (d2bNode.d2bDaemonNode {
        extra = { imports = extraModules; };
        writableStore = nodeShape == "writable-store";
      })
      {
        virtualisation.host.pkgs = pkgs;
      }
      laneGuestModule
    ];
  };
  guest = evaluated.config;
  toplevel = guest.system.build.toplevel;

  # The artifacts a launcher boots, the way NixOS's own VM module produces
  # them: the kernel and initrd that carry the system closure, and a root
  # disk in the format the module's run script builds. Real files, not a
  # copy of the toplevel symlink tree, so the image is what the lane boots.
  diskSizeMib = guest.virtualisation.diskSize;
  useBootLoader = guest.virtualisation.useBootLoader;

  # The two files the direct-boot shape is handed, named exactly as the VM
  # module's own run script names them: the kernel through the toplevel's
  # `kernel` link, and the initrd through `virtualisation.directBoot.initrd`,
  # so a node that redirects the payload redirects it here too. The build
  # below copies both into the image, because the manifest names them
  # relative to the image root and the image is what the lane boots.
  directBootKernel = "${toplevel}/kernel";
  directBootInitrd = guest.virtualisation.directBoot.initrd;

  # The registration file the guest's activation loads into its Nix
  # database, and the console list the VM module appends to a direct-boot
  # kernel command line. Both are read off the evaluated configuration
  # rather than restated, so the manifest carries what the guest declared.
  regInfo = guest.virtualisation.host.pkgs.closureInfo {
    rootPaths = guest.virtualisation.additionalPaths;
  };
  consoles = map (console: "console=${console}") guest.virtualisation.qemu.consoles;

  # The serial device the activation marker is written to, taken from the
  # console the guest was configured to log to.
  serialDevice = lib.head (lib.splitString "," (lib.head guest.virtualisation.qemu.consoles));

  # The bootloader shape boots from an installed system image rather than
  # from the empty ext4 image the direct-boot shape boots with, so its root
  # drive is a writable overlay on that image. It is built the way the VM
  # module builds its own: one MBR partition, a BIOS bootloader, no EFI
  # variables - the layout `selectPartitionTableLayout` picks for a node
  # that asks for a bootloader without asking for EFI.
  bootableSystemImage =
    import (pkgs.path + "/nixos/lib/make-disk-image.nix") {
      inherit pkgs;
      config = guest;
      inherit lib;
      additionalPaths = [ regInfo ];
      additionalSpace = "0M";
      copyChannel = false;
      diskSize = "auto";
      format = "qcow2";
      installBootLoader = true;
      label = "nixos";
      onlyNixStore = false;
      partitionTableType = "legacy";
      touchEFIVars = false;
    };

  # The direct-boot directives the VM module contributes to
  # `virtualisation.qemu.options` are re-declared under `boot` below with
  # their shell substitutions resolved, so they are removed from the option
  # list the launcher passes through verbatim. Everything a check
  # contributes - the state disk, a vsock device - stays in that list, in
  # its declared order.
  directBootFlags = [ "-kernel" "-initrd" "-append" ];
  isDirectBootFlag = option: lib.any (flag: lib.hasPrefix "${flag} " option) directBootFlags;
  extraOptions = builtins.filter (option: !(isDirectBootFlag option)) guest.virtualisation.qemu.options;

  # The networking the node declared. The VM module renders
  # `virtualisation.qemu.networkingOptions` in its own run script as a
  # separate group, alongside `qemu.options` rather than inside it, and the
  # launcher has to carry them for the same reason it carries the option
  # list: a guest booted without them has no network device at all, so the
  # kernel never loads the module that device needs and the daemon refuses
  # to start over exactly that. The one shell substitution in the default -
  # `"$QEMU_NET_OPTS"`, which the module leaves for an outer wrapper to fill
  # - is resolved to nothing here, the same way the kernel command line's
  # substitution is, because the launcher does not run a shell around it.
  networkingOptions = map
    (option: builtins.replaceStrings [ ''"$QEMU_NET_OPTS"'' ] [ "" ] option)
    guest.virtualisation.qemu.networkingOptions;

  # The root drive the launcher attaches. `virtualisation.qemu.drives`
  # entries are module submodules, so each is projected onto the fields the
  # launcher renders; `$NIX_DISK_IMAGE` is the VM module's placeholder for
  # the disk this image builds, and becomes the image's own `disk.qcow2`.
  driveFile = file: if file == ''"$NIX_DISK_IMAGE"'' then "disk.qcow2" else file;
  drives = map
    (drive: {
      name = drive.name or null;
      file = driveFile drive.file;
      format = drive.driveExtraOpts.format or null;
      cache = drive.driveExtraOpts.cache or "writeback";
      werror = drive.driveExtraOpts.werror or "report";
      bootIndex = drive.deviceExtraOpts.bootindex or null;
      serial = drive.deviceExtraOpts.serial or null;
      interface = guest.virtualisation.qemu.diskInterface;
    })
    guest.virtualisation.qemu.drives;

  # The host directories the guest mounts over 9p. A `$TMPDIR`-relative
  # source stays relative in the manifest and the launcher resolves it
  # against the working directory it owns, exactly as the VM module's run
  # script does.
  sharedDirectories = lib.mapAttrsToList
    (mountTag: share: {
      inherit mountTag;
      securityModel = share.securityModel;
      target = share.target;
      source = share.source;
    })
    (lib.filterAttrs (_: share: share ? source) guest.virtualisation.sharedDirectories);

  manifest = {
    schemaVersion = 1;
    system = pkgs.stdenv.hostPlatform.system;
    nodeShape = nodeShape;
    image = {
      disk = "disk.qcow2";
      diskFormat = "qcow2";
      inherit diskSizeMib;
      # Only the bootloader shape carries a backing system image; the
      # direct shape's root drive is self-contained.
      systemImage = if useBootLoader then "${bootableSystemImage}/nixos.qcow2" else null;
    };
    machine = {
      cores = guest.virtualisation.cores;
      memorySizeMib = guest.virtualisation.memorySize;
    };
    boot = {
      method = if useBootLoader then "bootloader" else "direct";
      kernel = if useBootLoader then null else "kernel";
      initrd = if useBootLoader then null else "initrd";
      append = if useBootLoader then
        null
      else
        "${builtins.readFile "${toplevel}/kernel-params"} init=${toplevel}/init regInfo=${regInfo}/registration ${lib.concatStringsSep " " consoles}";
    };
    inherit drives extraOptions networkingOptions;
    inherit sharedDirectories;
    activation = {
      readiness = "console-marker";
      contract = "d2b-daemon-acceptance";
      marker = activationMarker;
      inherit serialDevice;
      acceptanceUnitsFile = "/etc/d2b/daemon-acceptance-units";
      shape = nodeShape;
    };
    init = "${toplevel}/init";
    inherit toplevel;
    hostToolBundle = bazelHostTools.package;
    hostToolInventory = bazelHostTools.inventory;
    cloudHypervisorController = bazelHostTools.cloudHypervisorControllerPackage;
  };

  # The lane's own guest surface, layered on the re-homed node rather than
  # merged into it: the legacy fixtures boot these same nodes through the
  # nix test driver, which brings its own readiness, and a marker wired into
  # the shared module would reach their guests too.
  laneGuestModule =
    { config, lib, pkgs, ... }:
    {
      # The direct-boot shape gets its serial console from the `-append` the
      # VM module builds. The bootloader shape reads its command line off
      # the disk instead, so the same console list is declared as kernel
      # parameters for it - same consoles, same order, same primary.
      boot.kernelParams = lib.mkIf config.virtualisation.useBootLoader (
        map (console: "console=${console}") config.virtualisation.qemu.consoles
      );

      systemd.services.d2b-lane-activation = {
        description = "Report d2b host-integration lane guest activation";
        wantedBy = [ "multi-user.target" ];
        after = [ "network.target" ];
        unitConfig = {
          Type = "oneshot";
          RemainAfterExit = true;
        };
        serviceConfig.TimeoutStartSec = "${toString activationTimeoutSeconds}s";
        path = [
          pkgs.coreutils
          pkgs.gnugrep
          pkgs.systemd
        ];
        # The script is materialised by `writeShellScriptBin`, which embeds
        # this text verbatim: a Nix indented string escapes `${` and `''`,
        # and nothing else. A `$$` here is two literal dollar signs to the
        # shell, so `$$unit` is the script's own process id followed by
        # `unit` - a unit name that does not exist, polled forever. The
        # expansions below are therefore single-dollar.
        #
        # Every step of the contract is announced on the serial console, not
        # only the marker. The launcher's console is the only view it has of
        # the guest, so a guest that never activates is described by what it
        # last said it was waiting for - a unit name, or the unit's own
        # journal - rather than by the absence of a marker. Every line this
        # unit writes, the marker included, opens with a newline: the serial
        # console is a byte stream a login prompt shares, and a getty on the
        # same tty emits terminal queries - cursor position, erase display -
        # with no newline of its own, which otherwise lands them on the front
        # of this unit's next line.
        script = ''
          set -eu
          say() {
            printf '\nd2b-lane-activation: %s\n' "$1" >/dev/${serialDevice}
          }
          # A failed unit is reported at once, whoever it is, with its own
          # journal and the boot's failed set, rather than at the deadline.
          # The boot's failed set and not the polled unit's own state, because
          # a unit ordered after one that has not started never enters `failed`
          # at all: polling only its own state waits out the whole bound on a
          # guest whose failure is at a unit the poll has not reached.
          first_failed() {
            systemctl list-units --failed --no-legend --plain --no-pager \
              | head -n 1 | cut -d' ' -f1
          }
          report_failure() {
            say "$1 failed"
            journalctl -b --no-pager -o cat -u "$1" -n 40 >/dev/${serialDevice} 2>&1 || true
            systemctl --failed --no-pager --plain >/dev/${serialDevice} 2>&1 || true
            exit 1
          }
          # The units one unit is ordered against, requires, wants, and that
          # trigger it, read out of systemd's own view of the unit rather than
          # out of a list restated here: a guest that stalls is described by
          # the graph it stalls in, and that graph belongs to the node module.
          edges_of() {
            systemctl show "$1" --property=After --property=Requires \
              --property=Wants --property=Triggers --property=TriggeredBy \
              --value 2>/dev/null | tr ' ' '\n' | grep -v '^$' || true
          }
          unit_state() {
            echo "--- unit: $1"
            systemctl show "$1" --property=Id --property=LoadState \
              --property=ActiveState --property=SubState --property=Result \
              --property=UnitFileState --property=Description --property=After \
              --property=Requires --property=Wants --property=WantedBy \
              --property=Triggers --property=TriggeredBy 2>&1 || true
          }
          # What a reader of a failed lane needs to see a stall without
          # reproducing the run: the d2b slice of the unit list, the boot's
          # failed set, and every unit the contract is waiting on together
          # with each unit it is ordered against, required by, or waiting on.
          # A unit that never started and a unit ordered behind one that never
          # started are different failures, and only the graph distinguishes
          # them. Announced between two lines the launcher recognises, so the
          # state travels with the launcher's own activation failure instead of
          # being buried in the console's boot output.
          report_ordering() {
            stalled="$1"
            reason="$2"
            say "ordering state for $stalled ($reason)"
            {
              echo "=== d2b units on this boot ==="
              systemctl list-units --all --no-legend --plain --no-pager 'd2b*' 2>&1 || true
              echo "=== every unit this boot failed ==="
              systemctl list-units --failed --all --no-legend --plain --no-pager 2>&1 || true
              frontier="$stalled"
              seen=""
              while [ -n "$frontier" ]; do
                following=""
                for candidate in $frontier; do
                  case " $seen " in
                    *" $candidate "*) continue ;;
                  esac
                  seen="$seen $candidate"
                  unit_state "$candidate"
                  for edge in $(edges_of "$candidate"); do
                    case " $seen $following " in
                      *" $edge "*) ;;
                      *) following="$following $edge" ;;
                    esac
                  done
                done
                frontier="$following"
              done
            } >/dev/${serialDevice} 2>&1 || true
            say "end ordering state"
          }
          # Plain seconds. The systemd time span above carries an `s` because
          # that is a duration; shell arithmetic does not, and `1800s` is not
          # a number - it is a base the shell cannot read - so a deadline
          # written that way aborts this script on its first statement under
          # `set -e`, and the guest reports nothing at all.
          deadline=$(( $(date +%s) + ${toString activationTimeoutSeconds} ))
          units=""
          while read -r unit; do
            [ -n "$unit" ] || continue
            units="$units $unit"
            say "waiting for $unit"
            # A unit that has not activated for this long is described before
            # the launcher's own bound, not at it, because a report written
            # after the launcher has stopped reading the console is not a
            # report. The first report is unconditional; later ones repeat
            # only once the state may have moved on since.
            waiting_since=$(date +%s)
            reported_at=""
            until systemctl is-active --quiet "$unit"; do
              if failed=$(first_failed); [ -n "$failed" ]; then
                report_failure "$failed"
              fi
              now=$(date +%s)
              if [ $(( now - waiting_since )) -ge ${toString activationStallSeconds} ] &&
                { [ -z "$reported_at" ] || [ $(( now - reported_at )) -ge ${toString activationStallRepeatSeconds} ]; }; then
                report_ordering "$unit" "not active after $(( now - waiting_since ))s"
                reported_at="$now"
              fi
              if [ "$now" -ge "$deadline" ]; then
                say "$unit did not activate"
                journalctl -b --no-pager -o cat -u "$unit" -n 40 >/dev/${serialDevice} 2>&1 || true
                report_ordering "$unit" "activation deadline reached"
                exit 1
              fi
              sleep 1
            done
            say "$unit is active"
          done </etc/d2b/daemon-acceptance-units
          # A guest snapshotted before its random pool is initialised comes
          # back with a cold CRNG and a blocked getrandom(). The marker is
          # therefore also the snapshot precondition: it is written only
          # once the pool the lane will snapshot is ready.
          say "waiting for the random pool"
          until journalctl -b --no-pager -o cat | grep -q "random: crng init done"; do
            if failed=$(first_failed); [ -n "$failed" ]; then
              report_failure "$failed"
            fi
            if [ "$(date +%s)" -ge "$deadline" ]; then
              say "the random pool was not initialised"
              exit 1
            fi
            sleep 1
          done
          printf '\n%s\n' "${activationMarker} shape=${nodeShape} units=''${units# }" >/dev/${serialDevice}
        '';
      };
    };
in
if !(lib.elem nodeShape [ "daemon" "writable-store" ]) then
  throw ''
    d2b guest image: unknown node shape '${nodeShape}'.
      known shapes: daemon writable-store
  ''
else if missing != [ ] || unexpected != [ ] then
  throw ''
    d2b guest image: the staged Bazel host-tool bundle does not match the
    declared inventory.
      missing:    ${lib.concatStringsSep " " (if missing == [ ] then [ "(none)" ] else missing)}
      unexpected: ${lib.concatStringsSep " " (if unexpected == [ ] then [ "(none)" ] else unexpected)}
  ''
else
  # The filesystem UUID and the build clock are pinned so the same inputs
  # produce the same bytes: an image that changed hash on every build
  # would not be a cacheable graph output.
  let
    fakeTime = "1";
  in
  pkgs.runCommand "d2b-vm-guest-image-${nodeShape}" {
    nativeBuildInputs = [ pkgs.jq pkgs.e2fsprogs pkgs.qemu ];
  } ''
    mkdir -p "$out"

    export E2FSPROGS_FAKE_TIME=${fakeTime}
    qemu_img=${pkgs.qemu}/bin/qemu-img

    if ${if useBootLoader then "true" else "false"}; then
      # The bootloader shape: a writable qcow2 overlay on the installed
      # system image, sized so the overlay is at least as large as the disk
      # the node asked for, exactly as the VM module's run script sizes it.
      backing_mib=$(( $("$qemu_img" info ${bootableSystemImage}/nixos.qcow2 --output=json | ${pkgs.jq}/bin/jq -r '."virtual-size"') / 1024 / 1024 ))
      disk_mib=${toString diskSizeMib}
      if [ "$disk_mib" -gt "$backing_mib" ]; then
        overlay_mib="$disk_mib"
      else
        overlay_mib="$backing_mib"
      fi
      "$qemu_img" create -f qcow2 -F qcow2 \
        -b ${bootableSystemImage}/nixos.qcow2 \
        "$TMPDIR/disk.qcow2" "''${overlay_mib}M"
    else
      # The direct-boot shape's root disk, built the way the VM module's run
      # script builds it: an ext4 filesystem, converted to qcow2 so the lane
      # can snapshot and restore it. The system closure itself rides in the
      # initrd.
      "$qemu_img" create -f raw "$TMPDIR/root.raw" ${toString diskSizeMib}M
      ${pkgs.e2fsprogs}/bin/mkfs.ext4 -q -F -L nixos -U 00000000-0000-0000-0000-000000000001 "$TMPDIR/root.raw"
      "$qemu_img" convert -f raw -O qcow2 "$TMPDIR/root.raw" "$TMPDIR/disk.qcow2"
      rm -f "$TMPDIR/root.raw"

      # The direct-boot shape's kernel and initrd, copied in as real files
      # rather than named by store path. The manifest names them relative to
      # the image root, so an image that only pointed at the host's store
      # would declare files it does not carry, and the launcher would find
      # them missing at boot. A copy that fails fails the image here.
      cp -L ${directBootKernel} "$out/kernel"
      cp -L ${directBootInitrd} "$out/initrd"
    fi
    mv "$TMPDIR/disk.qcow2" "$out/disk.qcow2"

    # The manifest names the artifacts relative to the image root, the
    # exact host-tool package the closure was built against, the invocation
    # shape the lane reproduces per check, and the activation contract the
    # launcher waits for.
    cat >"$out/manifest.json" <<'JSON'
    ${builtins.toJSON manifest}
    JSON
    jq --sort-keys . "$out/manifest.json" >"$out/manifest.sorted"
    mv "$out/manifest.sorted" "$out/manifest.json"
  ''
