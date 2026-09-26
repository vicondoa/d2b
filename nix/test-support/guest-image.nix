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
# The guest is the node the current `vmChecks` fixtures boot, taken from
# the same `d2bDaemonNode` configuration and the same Bazel-built host-tool
# package, so a guest realized through this entry point and a guest
# realized through the legacy handoff are the same guest. Per-check module
# contributions arrive through `extraModules`.
#
# The output is one store path holding the guest's system closure and a
# manifest of what a launcher needs to boot it: the toplevel, the kernel
# and initrd entry points, the declared invocation shape, and the host-tool
# package the closure was built against.
{ pkgs, self, bazelHostTools, rawBundle, extraModules ? [ ] }:

let
  inherit (pkgs) lib;

  # Refuse an incomplete handoff before a guest closure is evaluated. The
  # host-tool package repeats this check when it is built, but that is the
  # wrong place to learn a binary is missing: by then the guest closure has
  # been realized.
  stagedEntries = builtins.filter (name: name != "." && name != "..")
    (lib.attrNames (builtins.readDir (/. + rawBundle)));
  missing = lib.filter (name: !(builtins.elem name stagedEntries)) bazelHostTools.inventory;
  unexpected = lib.filter (name: !(builtins.elem name bazelHostTools.inventory)) stagedEntries;

  d2bLib = import ../../tests/host-integration/lib.nix {
    self = self;
    inherit (pkgs) lib;
    hostToolBundle = bazelHostTools.package;
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
      (d2bLib.d2bDaemonNode { extra = { imports = extraModules; }; })
      {
        virtualisation.host.pkgs = pkgs;
      }
    ];
  };
  guest = evaluated.config;
  toplevel = guest.system.build.toplevel;

  # The artifacts a launcher boots, the way NixOS's own VM module produces
  # them: the kernel and initrd that carry the system closure, and a root
  # disk in the format the module's run script builds. Real files, not a
  # copy of the toplevel symlink tree, so the image is what the lane boots.
  diskSizeMib = guest.virtualisation.diskSize;
  manifest = {
    system = pkgs.stdenv.hostPlatform.system;
    kernel = "kernel";
    initrd = "initrd";
    disk = "disk.qcow2";
    diskFormat = "qcow2";
    init = "${toplevel}/init";
    toplevel = toplevel;
    hostToolBundle = bazelHostTools.package;
    hostToolInventory = bazelHostTools.inventory;
    cloudHypervisorController = bazelHostTools.cloudHypervisorControllerPackage;
    inherit (guest.virtualisation) cores diskSize memorySize;
    qemuOptions = guest.virtualisation.qemu.options;
  };
in
if missing != [ ] || unexpected != [ ] then
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
  pkgs.runCommand "d2b-vm-guest-image" {
    nativeBuildInputs = [ pkgs.jq pkgs.e2fsprogs pkgs.qemu ];
  } ''
    mkdir -p "$out"

    # The initrd carries the system closure, so the kernel and initrd are
    # the guest's real system. The toplevel's entries are store symlinks;
    # -L resolves them to files.
    cp -L ${toplevel}/kernel "$out/kernel"
    cp -L ${toplevel}/initrd "$out/initrd"

    # The root disk, built the way the VM module's own run script builds
    # it: an ext4 filesystem, converted to qcow2 so the lane can snapshot
    # and restore it.
    export E2FSPROGS_FAKE_TIME=${fakeTime}
    ${pkgs.qemu}/bin/qemu-img create -f raw "$TMPDIR/root.raw" ${toString diskSizeMib}M
    ${pkgs.e2fsprogs}/bin/mkfs.ext4 -q -F -L nixos -U 00000000-0000-0000-0000-000000000001 "$TMPDIR/root.raw"
    ${pkgs.qemu}/bin/qemu-img convert -f raw -O qcow2 "$TMPDIR/root.raw" "$out/disk.qcow2"
    rm -f "$TMPDIR/root.raw"

    # The manifest names the artifacts relative to the image root, the
    # exact host-tool package the closure was built against, and the
    # invocation shape the lane reproduces per check.
    cat >"$out/manifest.json" <<'JSON'
    ${builtins.toJSON manifest}
    JSON
    jq --sort-keys . "$out/manifest.json" >"$out/manifest.sorted"
    mv "$out/manifest.sorted" "$out/manifest.json"
  ''
