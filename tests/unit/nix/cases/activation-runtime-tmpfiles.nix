# Eval coverage for host activation posture that was moved to tmpfiles.
{ mkEval, lib, system, ... }:

let
  x86 = system == "x86_64-linux";
  fixture = { lib, ... }: {
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
    d2b.vms.corp-vm = {
      enable = true;
      env = "work";
      index = 10;
      ssh.user = "alice";
      audio.enable = x86;
      tpm.enable = true;
      graphics.enable = x86;
      graphics.videoSidecar = x86;
      config = { lib, ... }: {
        networking.hostName = lib.mkDefault "corp-vm";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
    d2b.daemonExperimental.enable = true;
  };
  qemuMediaFixture = { lib, ... }: {
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
    d2b.vms.media = {
      runtime.kind = "qemu-media";
      env = "work";
      index = 42;
      qemuMedia.source = {
        kind = "image-file";
        path = "/var/lib/d2b/images/installer.img";
        format = "raw";
      };
    };
    d2b.daemonExperimental.enable = true;
  };

  cfg = (mkEval [ fixture ]).config;
  qemuMediaCfg = (mkEval [ qemuMediaFixture ]).config;
  tmpfiles = cfg.systemd.tmpfiles.rules;
  qemuMediaTmpfiles = qemuMediaCfg.systemd.tmpfiles.rules;
  roleAclText = cfg.system.activationScripts.d2bRoleUidAcls.text or "";
  runtimePostureText = cfg.system.activationScripts.d2bRuntimeDirPosture.text or "";
  stateDirAclText = cfg.system.activationScripts.d2bStateDirAcl.text or "";
  vmStatePermsText = cfg.system.activationScripts.d2bVmStatePerms.text or "";
  tpmStatePermsText = cfg.system.activationScripts.d2bTpmStatePerms.text or "";
  netVmVarImgText = cfg.system.activationScripts.d2bNetVmVarImgPerms.text or "";
  storeSyncText = cfg.system.activationScripts.d2bStoreSync.text or "";
  audioStateDirsText = cfg.system.activationScripts.d2bAudioStateDirs.text or "";

  rulesForPath = path:
    builtins.filter (lib.hasInfix (" " + path + " ")) tmpfiles;
  stablePrincipal = principal:
    toString (50000 + lib.fromHexString (builtins.substring 0 6 (builtins.hashString "sha256" principal)));
  corpRunner = stablePrincipal "d2b-corp-vm-runner";
  corpGctlfs = stablePrincipal "d2b-corp-vm-gctlfs";

  noRawRuntimeDirMutation = text:
    lib.all (needle: !(lib.hasInfix needle text)) [
      "mkdir -p /run/d2b/vms/corp-vm"
      "chown d2bd:d2b /run/d2b/vms/corp-vm"
      "chmod 0750 /run/d2b/vms/corp-vm"
      "mkdir -p /run/d2b-gpu/corp-vm"
      "mkdir -p /run/d2b-video/corp-vm"
      "mkdir -p /run/d2b-wlproxy/corp-vm"
      "chown d2bd:d2b /run/d2b-gpu/corp-vm"
      "chown d2bd:d2b /run/d2b-video/corp-vm"
      "chown d2bd:d2b /run/d2b-wlproxy/corp-vm"
      "chmod 0750 /run/d2b-gpu/corp-vm"
      "chmod 0750 /run/d2b-video/corp-vm"
      "chmod 0750 /run/d2b-wlproxy/corp-vm"
      "mkdir -p /run/d2b/otel"
      "chown d2bd:d2b /run/d2b/otel"
      "chmod 0750 /run/d2b/otel"
    ];
in
{
  "activation-runtime-tmpfiles/state-root-posture" = {
    expr = lib.all (rule: builtins.elem rule tmpfiles) [
      "d /var/lib/d2b 0750 root d2bd -"
      "d /var/cache/d2b 0750 root d2bd -"
      "z /var/lib/d2b 0750 root d2bd -"
      "a+ /var/lib/d2b - - - - u:microvm:--x"
      "a+ /var/lib/d2b - - - - g:kvm:--x"
      "a+ /var/lib/d2b - - - - g:d2b:--x"
      "a+ /var/lib/d2b - - - - u:d2b-corp-vm-swtpm:--x"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/vm-state-dir" = {
    expr = lib.all (rule: builtins.elem rule tmpfiles) [
      "d /var/lib/d2b/vms/corp-vm 3770 d2bd users -"
      "z /var/lib/d2b/vms/corp-vm 3770 d2bd users -"
      "a+ /var/lib/d2b/vms/corp-vm - - - - u:d2b-corp-vm-swtpm:--x"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/run-vms-parent" = {
    expr = lib.all (rule: builtins.elem rule tmpfiles) ([
      "d /run/d2b/vms 0750 root d2b -"
      "z /run/d2b/vms 0750 root d2b -"
      "a+ /run/d2b/vms - - - - u:d2b-corp-vm-swtpm:--x"
      "a+ /run/d2b/vms - - - - u:${corpRunner}:--x"
      "a+ /run/d2b/vms - - - - u:${corpGctlfs}:--x"
    ] ++ lib.optionals x86 [
      "a+ /run/d2b/vms - - - - u:d2b-corp-vm-snd:--x"
      "a+ /run/d2b/vms - - - - u:d2b-corp-vm-gpu:--x"
    ]);
    expected = true;
  };

  "activation-runtime-tmpfiles/run-parent-mask-after-traversal-acls" = {
    expr =
      let
        runD2bRules = rulesForPath "/run/d2b";
      in
      builtins.elem "a+ /run/d2b - - - - u:d2bd:rwx" runD2bRules
      && (!x86 || builtins.elem "a+ /run/d2b - - - - u:d2b-corp-vm-snd:--x" runD2bRules)
      && builtins.elemAt runD2bRules ((builtins.length runD2bRules) - 2) == "a+ /run/d2b - - - - m::rwx"
      && lib.last runD2bRules == "a+ /run/d2b - - - - default:m::rwx";
    expected = true;
  };

  "activation-runtime-tmpfiles/activation-preserves-run-parent-daemon-mask" = {
    expr =
      lib.hasInfix ''setfacl -m "u:d2bd:rwx,g::r-x,m::rwx" /run/d2b 2>/dev/null || true'' runtimePostureText
      && !(lib.hasInfix ''setfacl -m "g::r-x,m::r-x" /run/d2b 2>/dev/null || true'' runtimePostureText);
    expected = true;
  };

  "activation-runtime-tmpfiles/gpu-parent" = {
    expr = !x86 || lib.all (rule: builtins.elem rule tmpfiles) [
      "d /run/d2b-gpu 0750 root d2b -"
      "z /run/d2b-gpu 0750 root d2b -"
      "a+ /run/d2b-gpu - - - - u:d2b-corp-vm-gpu:--x"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/video-parent" = {
    expr = !x86 || lib.all (rule: builtins.elem rule tmpfiles) [
      "d /run/d2b-video 0750 root d2b -"
      "z /run/d2b-video 0750 root d2b -"
      "a+ /run/d2b-video - - - - u:${corpRunner}:--x"
      "a+ /run/d2b-video - - - - u:d2b-corp-vm-video:--x"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/wlproxy-parent" = {
    expr = !x86 || lib.all (rule: builtins.elem rule tmpfiles) [
      "d /run/d2b-wlproxy 0750 root d2b -"
      "z /run/d2b-wlproxy 0750 root d2b -"
      "a+ /run/d2b-wlproxy - - - - u:d2b-corp-vm-gpu:--x"
      "a+ /run/d2b-wlproxy - - - - u:d2b-corp-vm-wlproxy:--x"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/run-vm-dir" = {
    expr = lib.all (rule: builtins.elem rule tmpfiles) ([
      "d /run/d2b/vms/corp-vm 1770 d2bd d2b -"
      "z /run/d2b/vms/corp-vm 1770 d2bd d2b -"
      "a+ /run/d2b/vms/corp-vm - - - - g::r-x"
      "a+ /run/d2b/vms/corp-vm - - - - default:g::r-x"
      "a+ /run/d2b/vms/corp-vm - - - - m::rwx"
      "a+ /run/d2b/vms/corp-vm - - - - default:m::rwx"
      "a+ /run/d2b/vms/corp-vm - - - - u:${corpRunner}:rwx"
      "a+ /run/d2b/vms/corp-vm - - - - default:u:${corpRunner}:rwx"
      "a+ /run/d2b/vms/corp-vm - - - - u:${corpGctlfs}:--x"
      "a+ /run/d2b/vms/corp-vm - - - - u:d2b-corp-vm-swtpm:rwx"
    ] ++ lib.optionals x86 [
      "a+ /run/d2b/vms/corp-vm - - - - u:d2b-corp-vm-gpu:rwx"
      "a+ /run/d2b/vms/corp-vm - - - - u:d2b-corp-vm-snd:rwx"
    ]);
    expected = true;
  };

  "activation-runtime-tmpfiles/run-guest-control-dir" = {
    expr = lib.all (rule: builtins.elem rule tmpfiles) [
      "d /run/d2b/vms/corp-vm/guest-control 0770 d2bd d2b -"
      "z /run/d2b/vms/corp-vm/guest-control 0770 d2bd d2b -"
      "a+ /run/d2b/vms/corp-vm/guest-control - - - - g::r-x"
      "a+ /run/d2b/vms/corp-vm/guest-control - - - - default:g::r-x"
      "a+ /run/d2b/vms/corp-vm/guest-control - - - - m::rwx"
      "a+ /run/d2b/vms/corp-vm/guest-control - - - - default:m::rwx"
      "a+ /run/d2b/vms/corp-vm/guest-control - - - - u:${corpRunner}:--x"
      "a+ /run/d2b/vms/corp-vm/guest-control - - - - u:${corpGctlfs}:rwx"
      "a+ /run/d2b/vms/corp-vm/guest-control - - - - default:u:${corpRunner}:rwx"
      "a+ /run/d2b/vms/corp-vm/guest-control - - - - default:u:${corpGctlfs}:rwx"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/run-store-sync-dir" = {
    expr = rulesForPath "/run/d2b/corp-vm";
    expected = [
      "d /run/d2b/corp-vm 0755 root root -"
    ];
  };

  "activation-runtime-tmpfiles/gpu-dir" = {
    expr = !x86 || lib.all (rule: builtins.elem rule tmpfiles) [
      "d /run/d2b-gpu/corp-vm 0770 d2bd d2b -"
      "z /run/d2b-gpu/corp-vm 0770 d2bd d2b -"
      "a+ /run/d2b-gpu/corp-vm - - - - g::r-x"
      "a+ /run/d2b-gpu/corp-vm - - - - m::rwx"
      "a+ /run/d2b-gpu/corp-vm - - - - default:m::rwx"
      "a+ /run/d2b-gpu/corp-vm - - - - u:d2b-corp-vm-gpu:rwx"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/video-dir" = {
    expr = !x86 || lib.all (rule: builtins.elem rule tmpfiles) [
      "d /run/d2b-video/corp-vm 0770 d2bd d2b -"
      "z /run/d2b-video/corp-vm 0770 d2bd d2b -"
      "a+ /run/d2b-video/corp-vm - - - - g::r-x"
      "a+ /run/d2b-video/corp-vm - - - - default:g::r-x"
      "a+ /run/d2b-video/corp-vm - - - - m::rwx"
      "a+ /run/d2b-video/corp-vm - - - - default:m::rwx"
      "a+ /run/d2b-video/corp-vm - - - - u:d2b-corp-vm-video:rwx"
      "a+ /run/d2b-video/corp-vm - - - - u:${corpRunner}:--x"
      "a+ /run/d2b-video/corp-vm - - - - default:u:${corpRunner}:rwx"
      "a+ /run/d2b-video/corp-vm - - - - default:u:d2b-corp-vm-video:rwx"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/wlproxy-dir" = {
    expr = !x86 || lib.all (rule: builtins.elem rule tmpfiles) [
      "d /run/d2b-wlproxy/corp-vm 0770 d2bd d2b -"
      "z /run/d2b-wlproxy/corp-vm 0770 d2bd d2b -"
      "a+ /run/d2b-wlproxy/corp-vm - - - - g::r-x"
      "a+ /run/d2b-wlproxy/corp-vm - - - - default:g::r-x"
      "a+ /run/d2b-wlproxy/corp-vm - - - - m::rwx"
      "a+ /run/d2b-wlproxy/corp-vm - - - - default:m::rwx"
      "a+ /run/d2b-wlproxy/corp-vm - - - - u:d2b-corp-vm-wlproxy:rwx"
      "a+ /run/d2b-wlproxy/corp-vm - - - - u:d2b-corp-vm-gpu:--x"
      "a+ /run/d2b-wlproxy/corp-vm - - - - default:u:d2b-corp-vm-gpu:rwx"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/qemu-media-runtime-dirs" = {
    expr = lib.all (rule: builtins.elem rule qemuMediaTmpfiles) [
      "a+ /run/d2b - - - - u:d2b-media-qemu-media:--x"
      "a+ /run/d2b/vms - - - - u:d2b-media-qemu-media:--x"
      "a+ /run/d2b-wlproxy - - - - u:d2b-media-qemu-media:--x"
      "a+ /run/d2b-wlproxy - - - - u:d2b-media-wlproxy:--x"
      "d /var/lib/d2b/vms/media/qemu-media 0750 d2b-media-qemu-media d2b-media-qemu-media -"
      "z /var/lib/d2b/vms/media/qemu-media 0750 d2b-media-qemu-media d2b-media-qemu-media -"
      "d /run/d2b/vms/media 0750 d2bd d2b -"
      "z /run/d2b/vms/media 0750 d2bd d2b -"
      "a+ /run/d2b/vms/media - - - - m::rwx"
      "a+ /run/d2b/vms/media - - - - u:d2b-media-qemu-media:rwx"
      "d /run/d2b-wlproxy/media 0770 d2bd d2b -"
      "z /run/d2b-wlproxy/media 0770 d2bd d2b -"
      "a+ /run/d2b-wlproxy/media - - - - m::rwx"
      "a+ /run/d2b-wlproxy/media - - - - u:d2b-media-wlproxy:rwx"
      "a+ /run/d2b-wlproxy/media - - - - u:d2b-media-qemu-media:--x"
      "a+ /run/d2b-wlproxy/media - - - - default:u:d2b-media-qemu-media:rwx"
    ];
    expected = true;
  };

  "activation-runtime-tmpfiles/store-view-live-dir" = {
    expr = rulesForPath "/var/lib/d2b/vms/corp-vm/store-view/live";
    expected = [
      "d /var/lib/d2b/vms/corp-vm/store-view/live 0755 d2bd users -"
      "z /var/lib/d2b/vms/corp-vm/store-view/live 0755 d2bd users -"
    ];
  };

  "activation-runtime-tmpfiles/role-acl-script-no-raw-runtime-dir-mutation" = {
    expr = noRawRuntimeDirMutation roleAclText;
    expected = true;
  };

  "activation-runtime-tmpfiles/role-acl-script-grants-run-vms-parent-traversal" = {
    expr =
      lib.hasInfix ''setfacl -m "u:$uid:x" /run/d2b 2>/dev/null || true'' roleAclText
      && lib.hasInfix ''setfacl -m "u:$uid:x" /run/d2b/vms 2>/dev/null || true'' roleAclText;
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
      !(lib.hasInfix "chown d2bd /var/lib/d2b/vms/corp-vm" vmStatePermsText)
      && !(lib.hasInfix "chgrp users /var/lib/d2b/vms/corp-vm" vmStatePermsText)
      && !(lib.hasInfix "chmod 3770 /var/lib/d2b/vms/corp-vm" vmStatePermsText);
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
    expr = !(lib.hasInfix "install -d -m 0755 /run/d2b/corp-vm" storeSyncText);
    expected = true;
  };

  "activation-runtime-tmpfiles/audio-state-dirs-no-root-posture" = {
    expr =
      !(lib.hasInfix "install -d -m 3770 -o d2bd -g users /var/lib/d2b/vms/corp-vm" audioStateDirsText)
      && !(lib.hasInfix "install -d -m 0750 -o d2bd -g d2b /var/lib/d2b/vms/corp-vm/state" audioStateDirsText)
      && !(lib.hasInfix "chown d2bd:d2b /var/lib/d2b/vms/corp-vm/state" audioStateDirsText);
    expected = true;
  };

  "activation-runtime-tmpfiles/runtime-posture-no-store-view-mkdir" = {
    expr = !(lib.hasInfix ''mkdir -p "$path"'' runtimePostureText);
    expected = true;
  };
}
