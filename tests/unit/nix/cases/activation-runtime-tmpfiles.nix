# Eval coverage for host activation posture that was moved to tmpfiles.
{ mkEval, lib, ... }:

let
  fixture = { lib, ... }: {
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
    nixling.vms.corp-vm = {
      enable = true;
      env = "work";
      index = 10;
      ssh.user = "alice";
      audio.enable = true;
      tpm.enable = true;
      graphics.enable = true;
      config = { lib, ... }: {
        networking.hostName = lib.mkDefault "corp-vm";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
    nixling.daemonExperimental.enable = true;
  };

  cfg = (mkEval [ fixture ]).config;
  tmpfiles = cfg.systemd.tmpfiles.rules;
  roleAclText = cfg.system.activationScripts.nixlingRoleUidAcls.text or "";
  runtimePostureText = cfg.system.activationScripts.nixlingRuntimeDirPosture.text or "";
  stateDirAclText = cfg.system.activationScripts.nixlingStateDirAcl.text or "";
  vmStatePermsText = cfg.system.activationScripts.nixlingVmStatePerms.text or "";
  tpmStatePermsText = cfg.system.activationScripts.nixlingTpmStatePerms.text or "";
  netVmVarImgText = cfg.system.activationScripts.nixlingNetVmVarImgPerms.text or "";
  storeSyncText = cfg.system.activationScripts.nixlingStoreSync.text or "";
  audioStateDirsText = cfg.system.activationScripts.nixlingAudioStateDirs.text or "";

  rulesForPath = path:
    builtins.filter (lib.hasInfix (" " + path + " ")) tmpfiles;

  noRawRuntimeDirMutation = text:
    lib.all (needle: !(lib.hasInfix needle text)) [
      "mkdir -p /run/nixling/vms/corp-vm"
      "chown nixlingd:nixling /run/nixling/vms/corp-vm"
      "chmod 0750 /run/nixling/vms/corp-vm"
      "mkdir -p /run/nixling-gpu/corp-vm"
      "mkdir -p /run/nixling-video/corp-vm"
      "mkdir -p /run/nixling-wlproxy/corp-vm"
      "chown nixlingd:nixling /run/nixling-gpu/corp-vm"
      "chown nixlingd:nixling /run/nixling-video/corp-vm"
      "chown nixlingd:nixling /run/nixling-wlproxy/corp-vm"
      "chmod 0750 /run/nixling-gpu/corp-vm"
      "chmod 0750 /run/nixling-video/corp-vm"
      "chmod 0750 /run/nixling-wlproxy/corp-vm"
      "mkdir -p /run/nixling/otel"
      "chown nixlingd:nixling /run/nixling/otel"
      "chmod 0750 /run/nixling/otel"
    ];
in
{
  "activation-runtime-tmpfiles/state-root-posture" = {
    expr = lib.all (rule: builtins.elem rule tmpfiles) [
      "d /var/lib/nixling 0750 root nixlingd -"
      "z /var/lib/nixling 0750 root nixlingd -"
      "a+ /var/lib/nixling - - - - u:microvm:--x"
      "a+ /var/lib/nixling - - - - g:kvm:--x"
      "a+ /var/lib/nixling - - - - g:nixling:--x"
      "a+ /var/lib/nixling - - - - u:nixling-corp-vm-swtpm:--x"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/vm-state-dir" = {
    expr = lib.all (rule: builtins.elem rule tmpfiles) [
      "d /var/lib/nixling/vms/corp-vm 3770 nixlingd users -"
      "z /var/lib/nixling/vms/corp-vm 3770 nixlingd users -"
      "a+ /var/lib/nixling/vms/corp-vm - - - - u:nixling-corp-vm-swtpm:--x"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/run-vms-parent" = {
    expr = lib.all (rule: builtins.elem rule tmpfiles) [
      "d /run/nixling/vms 0750 root nixling -"
      "z /run/nixling/vms 0750 root nixling -"
      "a+ /run/nixling/vms - - - - u:nixling-corp-vm-swtpm:--x"
      "a+ /run/nixling/vms - - - - u:nixling-corp-vm-snd:--x"
      "a+ /run/nixling/vms - - - - u:nixling-corp-vm-gpu:--x"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/gpu-parent" = {
    expr = lib.all (rule: builtins.elem rule tmpfiles) [
      "d /run/nixling-gpu 0750 root nixling -"
      "z /run/nixling-gpu 0750 root nixling -"
      "a+ /run/nixling-gpu - - - - u:nixling-corp-vm-gpu:--x"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/video-parent" = {
    expr = rulesForPath "/run/nixling-video";
    expected = [
      "d /run/nixling-video 0750 root nixling -"
      "z /run/nixling-video 0750 root nixling -"
    ];
  };

  "activation-runtime-tmpfiles/wlproxy-parent" = {
    expr = rulesForPath "/run/nixling-wlproxy";
    expected = [
      "d /run/nixling-wlproxy 0750 root nixling -"
      "z /run/nixling-wlproxy 0750 root nixling -"
      "a+ /run/nixling-wlproxy - - - - u:nixling-corp-vm-gpu:--x"
      "a+ /run/nixling-wlproxy - - - - u:nixling-corp-vm-wlproxy:--x"
    ];
  };

  "activation-runtime-tmpfiles/run-vm-dir" = {
    expr = lib.all (rule: builtins.elem rule tmpfiles) [
      "d /run/nixling/vms/corp-vm 0750 nixlingd nixling -"
      "z /run/nixling/vms/corp-vm 0750 nixlingd nixling -"
      "a+ /run/nixling/vms/corp-vm - - - - m::rwx"
      "a+ /run/nixling/vms/corp-vm - - - - default:m::rwx"
      "a+ /run/nixling/vms/corp-vm - - - - u:nixling-corp-vm-swtpm:rwx"
      "a+ /run/nixling/vms/corp-vm - - - - u:nixling-corp-vm-gpu:rwx"
      "a+ /run/nixling/vms/corp-vm - - - - u:nixling-corp-vm-snd:rwx"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/run-guest-control-dir" = {
    expr = lib.all (rule: builtins.elem rule tmpfiles) [
      "d /run/nixling/vms/corp-vm/guest-control 0750 nixlingd nixling -"
      "z /run/nixling/vms/corp-vm/guest-control 0750 nixlingd nixling -"
      "a+ /run/nixling/vms/corp-vm/guest-control - - - - m::rwx"
      "a+ /run/nixling/vms/corp-vm/guest-control - - - - default:m::rwx"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/run-store-sync-dir" = {
    expr = rulesForPath "/run/nixling/corp-vm";
    expected = [
      "d /run/nixling/corp-vm 0755 root root -"
    ];
  };

  "activation-runtime-tmpfiles/gpu-dir" = {
    expr = lib.all (rule: builtins.elem rule tmpfiles) [
      "d /run/nixling-gpu/corp-vm 0750 nixlingd nixling -"
      "z /run/nixling-gpu/corp-vm 0750 nixlingd nixling -"
      "a+ /run/nixling-gpu/corp-vm - - - - m::rwx"
      "a+ /run/nixling-gpu/corp-vm - - - - u:nixling-corp-vm-gpu:rwx"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/video-dir" = {
    expr = rulesForPath "/run/nixling-video/corp-vm";
    expected = [
      "d /run/nixling-video/corp-vm 0750 nixlingd nixling -"
      "z /run/nixling-video/corp-vm 0750 nixlingd nixling -"
    ];
  };

  "activation-runtime-tmpfiles/wlproxy-dir" = {
    expr = lib.all (rule: builtins.elem rule tmpfiles) [
      "d /run/nixling-wlproxy/corp-vm 0750 nixlingd nixling -"
      "z /run/nixling-wlproxy/corp-vm 0750 nixlingd nixling -"
      "a+ /run/nixling-wlproxy/corp-vm - - - - m::rwx"
      "a+ /run/nixling-wlproxy/corp-vm - - - - default:m::rwx"
      "a+ /run/nixling-wlproxy/corp-vm - - - - u:nixling-corp-vm-wlproxy:rwx"
      "a+ /run/nixling-wlproxy/corp-vm - - - - u:nixling-corp-vm-gpu:--x"
      "a+ /run/nixling-wlproxy/corp-vm - - - - default:u:nixling-corp-vm-gpu:rwx"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/store-view-live-dir" = {
    expr = rulesForPath "/var/lib/nixling/vms/corp-vm/store-view/live";
    expected = [
      "d /var/lib/nixling/vms/corp-vm/store-view/live 0755 nixlingd users -"
      "z /var/lib/nixling/vms/corp-vm/store-view/live 0755 nixlingd users -"
    ];
  };

  "activation-runtime-tmpfiles/role-acl-script-no-raw-runtime-dir-mutation" = {
    expr = noRawRuntimeDirMutation roleAclText;
    expected = true;
  };

  "activation-runtime-tmpfiles/state-dir-acl-script-no-static-sidecar-loop" = {
    expr =
      !(lib.hasInfix "for suffix in gpu swtpm audio video wlproxy qemu-media" stateDirAclText)
      && !(lib.hasInfix "u:microvm:--x" stateDirAclText)
      && !(lib.hasInfix "g:kvm:--x" stateDirAclText);
    expected = true;
  };

  "activation-runtime-tmpfiles/vm-state-perms-no-raw-root-posture" = {
    expr =
      !(lib.hasInfix "chown nixlingd /var/lib/nixling/vms/corp-vm" vmStatePermsText)
      && !(lib.hasInfix "chgrp users /var/lib/nixling/vms/corp-vm" vmStatePermsText)
      && !(lib.hasInfix "chmod 3770 /var/lib/nixling/vms/corp-vm" vmStatePermsText);
    expected = true;
  };

  "activation-runtime-tmpfiles/tpm-state-perms-tmpfiles-owned" = {
    expr = !(lib.hasInfix "setfacl" tpmStatePermsText);
    expected = true;
  };

  "activation-runtime-tmpfiles/net-var-img-broker-owned" = {
    expr = !(lib.hasInfix "ensure-regular-file" netVmVarImgText);
    expected = true;
  };

  "activation-runtime-tmpfiles/store-sync-no-run-dir-install" = {
    expr = !(lib.hasInfix "install -d -m 0755 /run/nixling/corp-vm" storeSyncText);
    expected = true;
  };

  "activation-runtime-tmpfiles/audio-state-dirs-no-root-posture" = {
    expr =
      !(lib.hasInfix "install -d -m 3770 -o nixlingd -g users /var/lib/nixling/vms/corp-vm" audioStateDirsText)
      && !(lib.hasInfix "install -d -m 0750 -o nixlingd -g nixling /var/lib/nixling/vms/corp-vm/state" audioStateDirsText)
      && !(lib.hasInfix "chown nixlingd:nixling /var/lib/nixling/vms/corp-vm/state" audioStateDirsText);
    expected = true;
  };

  "activation-runtime-tmpfiles/runtime-posture-no-store-view-mkdir" = {
    expr = !(lib.hasInfix ''mkdir -p "$path"'' runtimePostureText);
    expected = true;
  };
}
