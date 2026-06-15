# nix-unit cases migrated from tests/umask-roundtrip-eval.sh (group E).
#
# umask end-to-end eval round-trip: the `umask` field declared in
# nixos-modules/minijail-profiles.nix for each sidecar role (swtpm, gpu,
# video, audio) MUST propagate end-to-end through
#
#   minijail-profiles.nix → _bundle.minijailProfiles.<id>.roleProfile.umask
#   → processesJson.data.vms[*].nodes[*].profile.umask
#
# so a sidecar socket is created with 0o007 (decimal 7), NOT the broker's
# inherited 0o022. A silent pipeline drop in any layer would surface here.
#
# Evaluated against a synthesized nixosSystem with tpm.enable,
# graphics.enable, graphics.videoSidecar, and audio.enable all true — the
# minimal config that instantiates all four roles at once. Like the niri
# graphics case, this is x86_64-only by nature (the framework's
# checkVmPlatform gate refuses graphics/audio on non-x86_64 hosts).
{ mkEval, ... }:

let
  base = { lib, ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };
    nixling.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
    };
    nixling.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
    nixling.vms.umask-probe = {
      enable = true;
      env = "work";
      index = 10;
      ssh.user = "alice";
      tpm.enable = true;
      graphics.enable = true;
      graphics.videoSidecar = true;
      audio.enable = true;
      config = {
        networking.hostName = lib.mkDefault "umask-probe";
        users.users.alice = { isNormalUser = true; uid = 1000; };
        system.stateVersion = "25.11";
      };
    };
  };

  vms = (mkEval [ base ]).config.nixling._bundle.processesJson.data.vms;
  vm = builtins.head (builtins.filter (v: v.vm == "umask-probe") vms);
  nodes = vm.nodes;
  umaskOf = role:
    let matches = builtins.filter (n: n.role == role) nodes;
    in (builtins.head matches).profile.umask or null;
in
{
  "umask-roundtrip/swtpm" = {
    expr = umaskOf "swtpm";
    expected = 7;
  };
  "umask-roundtrip/gpu" = {
    expr = umaskOf "gpu";
    expected = 7;
  };
  "umask-roundtrip/video" = {
    expr = umaskOf "video";
    expected = 7;
  };
  "umask-roundtrip/audio" = {
    expr = umaskOf "audio";
    expected = 7;
  };
}
