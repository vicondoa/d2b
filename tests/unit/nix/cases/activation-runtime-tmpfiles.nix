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
  qemuMediaRoleAclText = qemuMediaCfg.system.activationScripts.d2bRoleUidAcls.text or "";
  runtimePostureText = cfg.system.activationScripts.d2bRuntimeDirPosture.text or "";
  stateDirAclText = cfg.system.activationScripts.d2bStateDirAcl.text or "";
  vmStatePermsText = cfg.system.activationScripts.d2bVmStatePerms.text or "";
  tpmStatePermsText = cfg.system.activationScripts.d2bTpmStatePerms.text or "";
  netVmVarImgText = cfg.system.activationScripts.d2bNetVmVarImgPerms.text or "";
  storeSyncText = cfg.system.activationScripts.d2bStoreSync.text or "";
  audioStateDirsText = cfg.system.activationScripts.d2bAudioStateDirs.text or "";
  storageJsonPaths = cfg.d2b._bundle.storageJson.data.paths;
  corpManifest = cfg.d2b.manifest."corp-vm" or null;

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
      builtins.elem "a+ /run/d2b - - - - g::r-x" runD2bRules
      && builtins.elem "a+ /run/d2b - - - - u:d2bd:rwx" runD2bRules
      && builtins.elem "a+ /run/d2b - - - - u:alice:--x" runD2bRules
      && (!x86 || builtins.elem "a+ /run/d2b - - - - u:d2b-corp-vm-snd:--x" runD2bRules)
      && builtins.elemAt runD2bRules ((builtins.length runD2bRules) - 2)
        == "a+ /run/d2b - - - - u:alice:--x"
      && lib.last runD2bRules == "a+ /run/d2b - - - - m::rwx";
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
      "a+ /run/d2b - - - - u:d2b-corp-vm-wlproxy:--x"
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

  "activation-runtime-tmpfiles/run-store-sync-pointer" = {
    expr =
      let
        rules = rulesForPath "/run/d2b/corp-vm/next-generation";
      in
      builtins.length rules == 1
      && lib.hasPrefix "L+ /run/d2b/corp-vm/next-generation - - - - /nix/store/" (lib.head rules)
      && lib.hasSuffix "-d2b-corp-vm-generation" (lib.head rules);
    expected = true;
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
      "a+ /run/d2b - - - - u:d2b-media-wlproxy:--x"
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

  "activation-runtime-tmpfiles/qemu-media-role-acl-script-repairs-run-write-mask" = {
    expr =
      lib.hasInfix ''qemu_media_acl_mask_repair=0'' qemuMediaRoleAclText
      && lib.hasInfix ''qemu_media_acl_mask_repair=1'' qemuMediaRoleAclText
      && lib.hasInfix ''if [ "$qemu_media_acl_mask_repair" = "1" ]; then'' qemuMediaRoleAclText
      && lib.hasInfix ''setfacl -m "m::rwx" /run/d2b/vms/media 2>/dev/null || true'' qemuMediaRoleAclText
      && !(lib.hasInfix ''setfacl -d -m "m::rwx" /run/d2b/vms/media'' qemuMediaRoleAclText)
      && !(lib.hasInfix ''setfacl -m "m::rwx" /var/lib/d2b/vms/media'' qemuMediaRoleAclText);
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

  "activation-runtime-tmpfiles/store-sync-creates-only-run-leaf" = {
    expr =
      lib.hasInfix "mkdir /run/d2b/corp-vm" storeSyncText
      && lib.hasInfix "refusing symlinked runtime leaf /run/d2b/corp-vm" storeSyncText
      && lib.hasInfix "refusing non-directory runtime leaf /run/d2b/corp-vm" storeSyncText
      && lib.hasInfix "enforce-dir-posture --path /run/d2b/corp-vm --uid 0 --gid 0 --mode 0755" storeSyncText
      && !(lib.hasInfix "chown root:root /run/d2b/corp-vm" storeSyncText)
      && !(lib.hasInfix "chmod 0755 /run/d2b/corp-vm" storeSyncText)
      && !(lib.hasInfix "mkdir -p /run/d2b/corp-vm" storeSyncText)
      && !(lib.hasInfix "install -d -m 0755 -o root -g root /run/d2b/corp-vm" storeSyncText);
    expected = true;
  };

  "activation-runtime-tmpfiles/store-sync-defers-missing-run-parent" = {
    expr =
      lib.hasInfix "if [ -L /run/d2b ]; then" storeSyncText
      && lib.hasInfix "refusing symlinked runtime parent /run/d2b" storeSyncText
      && lib.hasInfix "elif [ ! -e /run/d2b ]; then" storeSyncText
      && lib.hasInfix "deferring pointer publication to systemd-tmpfiles" storeSyncText
      && lib.hasInfix "elif [ ! -d /run/d2b ]; then" storeSyncText
      && lib.hasInfix "refusing non-directory runtime parent /run/d2b" storeSyncText
      && !(lib.hasInfix "parent missing; tmpfiles/host runtime posture did not run" storeSyncText);
    expected = true;
  };

  "activation-runtime-tmpfiles/audio-state-dirs-no-root-posture" = {
    expr =
      !(lib.hasInfix "install -d -m 3770 -o d2bd -g users /var/lib/d2b/vms/corp-vm" audioStateDirsText)
      && !(lib.hasInfix "install -d -m 0750 -o d2bd -g d2b /var/lib/d2b/vms/corp-vm/state" audioStateDirsText)
      && !(lib.hasInfix "chown d2bd:d2b /var/lib/d2b/vms/corp-vm/state" audioStateDirsText);
    expected = true;
  };

  # Audio ACL grants are now in tmpfiles (a+ rules) not activation scripts:
  # this eliminates the fresh-boot race where setfacl could run before the
  # audio-state.json file was created by tmpfiles.
  "activation-runtime-tmpfiles/audio-acls-in-tmpfiles-not-activation" = {
    expr = !x86 || (
      !(lib.hasInfix "setfacl" audioStateDirsText)
      && lib.all (rule: builtins.elem rule tmpfiles) [
        "a+ /var/lib/d2b/vms/corp-vm - - - - g:d2b:--x"
        "a+ /var/lib/d2b/vms/corp-vm/state - - - - u:d2b-corp-vm-gpu:r-x"
        "a+ /var/lib/d2b/vms/corp-vm/state/audio-state.json - - - - u:d2b-corp-vm-gpu:r--"
        # Default ACL ensures any replacement inode created by atomic rename
        # (write temp file in state/, rename to audio-state.json) inherits
        # GPU read access on the new inode.
        "a+ /var/lib/d2b/vms/corp-vm/state - - - - default:u:d2b-corp-vm-gpu:r--"
        "a+ /var/lib/d2b/vms/corp-vm/state - - - - default:m::r--"
      ]
    );
    expected = true;
  };

  # Audio lock is at the canonical /run/d2b/locks/ location (ADR 0034)
  # and is NOT at the old /run/d2b/audio-<vm>.lock root location.
  "activation-runtime-tmpfiles/audio-lock-in-locks-dir" = {
    expr = !x86 || (
      builtins.elem "f /run/d2b/locks/audio-corp-vm.lock 0660 root d2b -" tmpfiles
      && !(builtins.elem "f /run/d2b/audio-corp-vm.lock 0660 root d2b -" tmpfiles)
    );
    expected = true;
  };

  # d2b group traversal on /run/d2b/locks is granted when any audio VM
  # is enabled, so d2b members can reach the per-VM advisory lock files.
  "activation-runtime-tmpfiles/audio-locks-dir-d2b-traversal" = {
    expr = !x86 || builtins.elem "a+ /run/d2b/locks - - - - g:d2b:--x" tmpfiles;
    expected = true;
  };

  "activation-runtime-tmpfiles/runtime-posture-no-store-view-mkdir" = {
    expr = !(lib.hasInfix ''mkdir -p "$path"'' runtimePostureText);
    expected = true;
  };

  # audioService is always null: the d2b-<vm>-snd.service systemd unit is
  # retired; the audio sidecar runs as a broker-spawned runner.
  "activation-runtime-tmpfiles/manifest-audio-service-null" = {
    expr = !x86 || (corpManifest != null && corpManifest.audioService == null);
    expected = true;
  };

  # storage-json declares the per-VM audio state directory path.
  "activation-runtime-tmpfiles/storage-json-has-audio-state-dir" = {
    expr = !x86 || builtins.any
      (p: p.id == "path:vm-audio-state-dir:corp-vm"
        && p.pathTemplate == "/var/lib/d2b/vms/corp-vm/state")
      storageJsonPaths;
    expected = true;
  };

  # storage-json declares the per-VM audio state file path.
  "activation-runtime-tmpfiles/storage-json-has-audio-state-file" = {
    expr = !x86 || builtins.any
      (p: p.id == "path:vm-audio-state-file:corp-vm"
        && p.pathTemplate == "/var/lib/d2b/vms/corp-vm/state/audio-state.json")
      storageJsonPaths;
    expected = true;
  };

  # storage-json declares the per-VM audio lock at the canonical locks/ path.
  "activation-runtime-tmpfiles/storage-json-has-audio-lock-in-locks-dir" = {
    expr = !x86 || builtins.any
      (p: p.id == "path:vm-audio-lock:corp-vm"
        && p.pathTemplate == "/run/d2b/locks/audio-corp-vm.lock")
      storageJsonPaths;
    expected = true;
  };
}
