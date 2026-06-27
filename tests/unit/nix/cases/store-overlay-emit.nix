# nix-unit cases migrated from tests/store-overlay-emit-eval.sh (group E).
#
# v1.2 DiskInit plan-op emit gate: processes-json.nix emits a `DiskInit`
# plan-op in the cloud-hypervisor node's `planOps` field when the guest VM
# config sets `microvm.writableStoreOverlay` to a non-null path, and emits
# none when the option is left at its null default.
#
# Enabled case (microvm.writableStoreOverlay = "/nix/.rw-store"):
#   1. CH node has exactly one planOps entry.
#   2. entry.kind = "diskInit".
#   3. entry.targetPath ends with /<vm>/store-overlay.img.
#   4. entry.sizeBytes = 1 GiB (1073741824).
#   5. entry.mode = 384 (0o600 decimal).
#   6. entry.ifAbsent = true.
#   7. entry.ownerUid is a positive integer (runner profile uid).
#   8. entry.ownerGid is a positive integer (runner profile gid).
# Disabled case (option unset): planOps absent/empty.
#
# Uses `mkEval` to render the real per-VM processesJson.data; these plain
# (non-graphics) VMs evaluate on any host platform.
{ mkEval, lib, ... }:

let
  mkBase = vmName: index: extraConfig: { lib, ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
    };
    d2b.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    d2b.vms.${vmName} = {
      enable = true;
      env = "work";
      index = index;
      ssh.user = "alice";
      config = {
        networking.hostName = lib.mkDefault vmName;
        users.users.alice = { isNormalUser = true; uid = 1000; };
        system.stateVersion = "25.11";
      } // extraConfig;
    };
  };

  chNodeFor = vmName: sys:
    let
      vms = sys.config.d2b._bundle.processesJson.data.vms;
      vm = builtins.head (builtins.filter (v: v.vm == vmName) vms);
    in
    builtins.head (builtins.filter (n: n.id == "cloud-hypervisor") vm.nodes);

  enabledCh = chNodeFor "overlay-probe"
    (mkEval [ (mkBase "overlay-probe" 11 { microvm.writableStoreOverlay = "/nix/.rw-store"; }) ]);
  enabledOps = enabledCh.planOps or [ ];
  firstOp = builtins.head enabledOps;

  disabledCh = chNodeFor "no-overlay-probe"
    (mkEval [ (mkBase "no-overlay-probe" 12 { }) ]);
in
{
  "store-overlay-emit/planops-count" = {
    expr = builtins.length enabledOps;
    expected = 1;
  };
  "store-overlay-emit/first-op-kind" = {
    expr = firstOp.kind;
    expected = "diskInit";
  };
  "store-overlay-emit/first-op-target-suffix" = {
    expr = lib.hasSuffix "/overlay-probe/store-overlay.img" firstOp.targetPath;
    expected = true;
  };
  "store-overlay-emit/first-op-size-bytes" = {
    expr = firstOp.sizeBytes;
    expected = 1073741824;
  };
  "store-overlay-emit/first-op-mode" = {
    expr = firstOp.mode;
    expected = 384;
  };
  "store-overlay-emit/first-op-if-absent" = {
    expr = firstOp.ifAbsent;
    expected = true;
  };
  "store-overlay-emit/first-op-owner-uid-positive" = {
    expr = firstOp.ownerUid > 0;
    expected = true;
  };
  "store-overlay-emit/first-op-owner-gid-positive" = {
    expr = firstOp.ownerGid > 0;
    expected = true;
  };
  "store-overlay-emit/disabled-no-planops" = {
    expr = (disabledCh.planOps or [ ]) == [ ];
    expected = true;
  };
}
