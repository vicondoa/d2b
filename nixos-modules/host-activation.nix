# Activation scripts for d2b.
#
# Historical one-shot repairs were removed from this module. If you're
# upgrading from a pre-public version of d2b, see CHANGELOG.md for
# the manual migration steps.
#
#   * d2bSbctlBackup    — host-specific (maintainer's sbctl pipeline);
#                             no public framework concern. Move
#                             *-backup.tar.gz files out of $HOME by hand
#                             if you ever ran the maintainer setup.
#   * d2bStoreChownRepair — one-shot fix for a past chown bug that
#                             leaked group=kvm into /nix/store inodes via
#                             the per-VM hardlink farm. Run the repair
#                             script from the historical /etc/nixos commit
#                             once, then forget about it. New installs are
#                             unaffected.
#   * d2bMigrateState   — one-shot renamer (/var/lib/microvms →
#                             /var/lib/d2b/vms, plus /var/lib/swtpm
#                             → vms/<vm>/swtpm). New installs land on
#                             the new layout from the start. Existing
#                             consumers should use the migration script.
#
# What remains here
#   - d2bVmStatePerms       — per-graphics-VM ACLs on the state dir so the
#                                 d2b-<vm>-gpu sidecar user (not
#                                 microvm) can read/write it. Directory
#                                 posture is tmpfiles-owned.
#   - d2bNetVmVarImgPerms   — compatibility tombstone; net-VM var.img
#                                 creation/posture is broker DiskInit-owned.
#   - d2bMigrateOwnership   — repair orphan swtpm-state UIDs after
#                                 service-user renames, gated on
#                                 `tpm.enable` and skipped for running VMs.
{ config, pkgs, lib, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib pkgs; };
  normalNixosVms = d2bLib.normalNixosVms cfg.vms;
  qemuMediaVms = d2bLib.qemuMediaVms cfg.vms;
  roleAclVms = normalNixosVms // qemuMediaVms;
  prebuilt =
    if cfg.site.usePrebuiltHostTools
    then import ./prebuilt-packages.nix { inherit pkgs lib; }
    else { };

  # Build the d2b-activation-helper binary (defined in
  # packages/d2b-host/src/bin/d2b-activation-helper.rs)
  # as a derivation here so activation scripts can call it via a
  # store path that's valid BEFORE
  # /run/current-system/sw/bin/<name> is populated (chicken-and-
  # egg during the very first activation). Each activation
  # snippet references `${activationHelper}` to get the absolute
  # store-path of the binary.
  packagesSrc = d2bLib.cleanRustPackagesSource ../packages;
  cargoLock = {
    lockFile = ../packages/Cargo.lock;
    outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
  };
  activationHelperSourcePackage = pkgs.rustPlatform.buildRustPackage {
    pname = "d2b-activation-helper";
    version = "0.0.0-bootstrap";
    src = packagesSrc;
    inherit cargoLock;
    cargoBuildFlags = [ "--package" "d2b-host" "--bin" "d2b-activation-helper" ];
    doCheck = false;
    postPatch = ''
      mkdir -p .cargo
      cat > .cargo/config.toml <<EOF
[build]
rustc-wrapper = ""
EOF
      rm -f .cargo/rustc-wrapper.sh
    '';
    installPhase = ''
      runHook preInstall
      install -Dm755 target/x86_64-unknown-linux-gnu/release/d2b-activation-helper $out/bin/d2b-activation-helper 2>/dev/null \
        || install -Dm755 target/release/d2b-activation-helper $out/bin/d2b-activation-helper
      runHook postInstall
    '';
  };
  activationHelperPackage = if prebuilt ? "d2b-activation-helper" then prebuilt."d2b-activation-helper" else activationHelperSourcePackage;
  activationHelper = "${activationHelperPackage}/bin/d2b-activation-helper";
  groupMigrationHelperPackage = pkgs.rustPlatform.buildRustPackage {
    pname = "d2b-host-activation-helper";
    version = "0.0.0-bootstrap";
    src = packagesSrc;
    cargoBuildFlags = [ "--package" "d2b-host-activation-helper" ];
    inherit cargoLock;
    doCheck = false;
    postPatch = ''
      mkdir -p .cargo
      cat > .cargo/config.toml <<EOF
[build]
rustc-wrapper = ""
EOF
      rm -f .cargo/rustc-wrapper.sh
    '';
  };
  groupMigrationHelper = "${groupMigrationHelperPackage}/bin/d2b-host-activation-helper";
  legacyLauncherGid = config.users.groups.d2b-launcher.gid or null;
  legacyLaunchersGid = config.users.groups.d2b-launchers.gid or null;
  legacyGidsArg = lib.concatStringsSep "," (
    builtins.filter (g: g != null)
      [ (if legacyLauncherGid == null then null else toString legacyLauncherGid)
        (if legacyLaunchersGid == null then null else toString legacyLaunchersGid)
      ]);
  tmpfilesDir = path: mode: user: group: [
    "d ${path} ${mode} ${user} ${group} -"
    "z ${path} ${mode} ${user} ${group} -"
  ];
  tmpfilesAcl = path: acl: [
    "a+ ${path} - - - - ${acl}"
  ];
  runtimeLeafDir = path: mode: user: group:
    tmpfilesDir path mode user group
    ++ tmpfilesAcl path "g::r-x"
    ++ tmpfilesAcl path "default:g::r-x";
  runtimeAclMask = path: [
    "a+ ${path} - - - - m::rwx"
    "a+ ${path} - - - - default:m::rwx"
  ];
  runtimeAclUser = path: principal: perms: tmpfilesAcl path "u:${principal}:${perms}";
  runtimeDefaultAclUser = path: principal: perms: tmpfilesAcl path "default:u:${principal}:${perms}";
  stablePrincipal = principal: toString (d2bLib.stablePrincipalId principal);
  perVmNamedStateTraversalAcls = lib.concatLists [
    (lib.concatLists (lib.mapAttrsToList (name: _: tmpfilesAcl "/var/lib/d2b" "u:d2b-${name}-gpu:--x")
      (lib.filterAttrs (_: vm: vm.graphics.enable) normalNixosVms)))
    (lib.concatLists (lib.mapAttrsToList (name: _: tmpfilesAcl "/var/lib/d2b" "u:d2b-${name}-video:--x")
      (lib.filterAttrs (_: vm: vm.graphics.enable && vm.graphics.videoSidecar) normalNixosVms)))
    (lib.concatLists (lib.mapAttrsToList (name: _: tmpfilesAcl "/var/lib/d2b" "u:d2b-${name}-wlproxy:--x")
      ((lib.filterAttrs (_: vm: vm.graphics.enable) normalNixosVms) // qemuMediaVms)))
    (lib.concatLists (lib.mapAttrsToList (name: _: tmpfilesAcl "/var/lib/d2b" "u:d2b-${name}-snd:--x")
      (lib.filterAttrs (_: vm: vm.audio.enable) normalNixosVms)))
    (lib.concatLists (lib.mapAttrsToList (name: _: tmpfilesAcl "/var/lib/d2b" "u:d2b-${name}-swtpm:--x")
      (lib.filterAttrs (_: vm: vm.tpm.enable) normalNixosVms)))
    (lib.concatLists (lib.mapAttrsToList (name: _: tmpfilesAcl "/var/lib/d2b" "u:d2b-${name}-qemu-media:--x")
      qemuMediaVms))
  ];
  perVmRuntimeTraversalAcls = lib.concatLists (lib.mapAttrsToList
    (name: vm:
      let
        runnerPrincipal = stablePrincipal "d2b-${name}-runner";
        gctlfsPrincipal = stablePrincipal "d2b-${name}-gctlfs";
      in
      lib.concatLists [
        (runtimeAclUser "/run/d2b" runnerPrincipal "--x")
        (runtimeAclUser "/run/d2b" gctlfsPrincipal "--x")
        (runtimeAclUser "/run/d2b/vms" runnerPrincipal "--x")
        (runtimeAclUser "/run/d2b/vms" gctlfsPrincipal "--x")
        (lib.optionals vm.tpm.enable (runtimeAclUser "/run/d2b" "d2b-${name}-swtpm" "--x"))
        (lib.optionals vm.tpm.enable (runtimeAclUser "/run/d2b/vms" "d2b-${name}-swtpm" "--x"))
        (lib.optionals vm.audio.enable (runtimeAclUser "/run/d2b" "d2b-${name}-snd" "--x"))
        (lib.optionals vm.audio.enable (runtimeAclUser "/run/d2b/vms" "d2b-${name}-snd" "--x"))
        (lib.optionals vm.graphics.enable (runtimeAclUser "/run/d2b" "d2b-${name}-gpu" "--x"))
        (lib.optionals vm.graphics.enable (runtimeAclUser "/run/d2b/vms" "d2b-${name}-gpu" "--x"))
        (lib.optionals vm.graphics.enable (runtimeAclUser "/run/d2b-gpu" "d2b-${name}-gpu" "--x"))
        (lib.optionals vm.graphics.enable (runtimeAclUser "/run/d2b-wlproxy" "d2b-${name}-gpu" "--x"))
        (lib.optionals vm.graphics.enable (runtimeAclUser "/run/d2b-wlproxy" "d2b-${name}-wlproxy" "--x"))
        (lib.optionals (vm.graphics.enable && vm.graphics.videoSidecar) (runtimeAclUser "/run/d2b-video" runnerPrincipal "--x"))
        (lib.optionals (vm.graphics.enable && vm.graphics.videoSidecar) (runtimeAclUser "/run/d2b-video" "d2b-${name}-video" "--x"))
      ])
    normalNixosVms);
  perQemuMediaRuntimeTraversalAcls = lib.concatLists (lib.mapAttrsToList
    (name: _:
      let
        qemuMediaPrincipal = "d2b-${name}-qemu-media";
        wlproxyPrincipal = "d2b-${name}-wlproxy";
      in
      lib.concatLists [
        (runtimeAclUser "/run/d2b" qemuMediaPrincipal "--x")
        (runtimeAclUser "/run/d2b/vms" qemuMediaPrincipal "--x")
        (runtimeAclUser "/run/d2b-wlproxy" qemuMediaPrincipal "--x")
        (runtimeAclUser "/run/d2b-wlproxy" wlproxyPrincipal "--x")
      ])
    qemuMediaVms);
  perNormalVmPostureTmpfiles = lib.concatLists (lib.mapAttrsToList
    (name: vm:
      let
        vmRunDir = "/run/d2b/vms/${name}";
        guestControlRunDir = "${vmRunDir}/guest-control";
        gpuRunDir = "/run/d2b-gpu/${name}";
        videoRunDir = "/run/d2b-video/${name}";
        wlproxyRunDir = "/run/d2b-wlproxy/${name}";
        runnerPrincipal = stablePrincipal "d2b-${name}-runner";
        gctlfsPrincipal = stablePrincipal "d2b-${name}-gctlfs";
      in
      lib.concatLists [
      (tmpfilesDir "/var/lib/d2b/vms/${name}" "3770" "d2bd" "users")
      (runtimeLeafDir vmRunDir "1770" "d2bd" "d2b")
      (runtimeLeafDir guestControlRunDir "0770" "d2bd" "d2b")
      (runtimeLeafDir gpuRunDir "0770" "d2bd" "d2b")
      (runtimeLeafDir videoRunDir "0770" "d2bd" "d2b")
      (runtimeLeafDir wlproxyRunDir "0770" "d2bd" "d2b")
      (tmpfilesDir "/var/lib/d2b/vms/${name}/store-view" "0755" "d2bd" "users")
      (tmpfilesDir "/var/lib/d2b/vms/${name}/store-view/live" "0755" "d2bd" "users")
      (tmpfilesDir "/var/lib/d2b/vms/${name}/store-view/meta" "0755" "d2bd" "users")
      (runtimeAclMask vmRunDir)
      (runtimeAclUser vmRunDir runnerPrincipal "rwx")
      (runtimeAclUser vmRunDir gctlfsPrincipal "--x")
      (runtimeDefaultAclUser vmRunDir runnerPrincipal "rwx")
      (runtimeAclMask guestControlRunDir)
      (runtimeAclUser guestControlRunDir runnerPrincipal "--x")
      (runtimeAclUser guestControlRunDir gctlfsPrincipal "rwx")
      (runtimeDefaultAclUser guestControlRunDir runnerPrincipal "rwx")
      (runtimeDefaultAclUser guestControlRunDir gctlfsPrincipal "rwx")
      (lib.optionals vm.tpm.enable (tmpfilesAcl "/var/lib/d2b/vms/${name}" "u:d2b-${name}-swtpm:--x"))
      (lib.optionals vm.tpm.enable (runtimeAclUser vmRunDir "d2b-${name}-swtpm" "rwx"))
      (lib.optionals vm.graphics.enable (tmpfilesAcl "/var/lib/d2b/vms/${name}" "u:d2b-${name}-gpu:rwx"))
      (lib.optionals vm.graphics.enable (tmpfilesAcl "/var/lib/d2b/vms/${name}" "default:u:d2b-${name}-gpu:rw"))
      (lib.optionals vm.graphics.enable (tmpfilesAcl "/var/lib/d2b/vms/${name}" "default:g::r-x"))
      (lib.optionals vm.graphics.enable (runtimeAclUser vmRunDir "d2b-${name}-gpu" "rwx"))
      (lib.optionals vm.graphics.enable (runtimeAclUser gpuRunDir "d2b-${name}-gpu" "rwx"))
      (lib.optionals vm.graphics.enable (runtimeAclMask gpuRunDir))
      (lib.optionals vm.graphics.enable (runtimeAclUser wlproxyRunDir "d2b-${name}-wlproxy" "rwx"))
      (lib.optionals vm.graphics.enable (runtimeAclUser wlproxyRunDir "d2b-${name}-gpu" "--x"))
      (lib.optionals vm.graphics.enable (runtimeDefaultAclUser wlproxyRunDir "d2b-${name}-gpu" "rwx"))
      (lib.optionals vm.graphics.enable (runtimeAclMask wlproxyRunDir))
      (lib.optionals (vm.graphics.enable && vm.graphics.videoSidecar) (runtimeAclUser videoRunDir "d2b-${name}-video" "rwx"))
      (lib.optionals (vm.graphics.enable && vm.graphics.videoSidecar) (runtimeAclUser videoRunDir runnerPrincipal "--x"))
      (lib.optionals (vm.graphics.enable && vm.graphics.videoSidecar) (runtimeDefaultAclUser videoRunDir runnerPrincipal "rwx"))
      (lib.optionals (vm.graphics.enable && vm.graphics.videoSidecar) (runtimeDefaultAclUser videoRunDir "d2b-${name}-video" "rwx"))
      (lib.optionals (vm.graphics.enable && vm.graphics.videoSidecar) (runtimeAclMask videoRunDir))
      (lib.optionals vm.audio.enable (runtimeAclUser vmRunDir "d2b-${name}-snd" "rwx"))
    ])
    normalNixosVms);
  perQemuMediaPostureTmpfiles = lib.concatLists (lib.mapAttrsToList
    (name: _:
      let
        qemuMediaPrincipal = "d2b-${name}-qemu-media";
        wlproxyPrincipal = "d2b-${name}-wlproxy";
        vmRunDir = "/run/d2b/vms/${name}";
        wlproxyRunDir = "/run/d2b-wlproxy/${name}";
      in
      lib.concatLists [
      (tmpfilesDir "/var/lib/d2b/vms/${name}/qemu-media" "0750" "d2b-${name}-qemu-media" "d2b-${name}-qemu-media")
      (runtimeLeafDir vmRunDir "0750" "d2bd" "d2b")
      (runtimeAclMask vmRunDir)
      (runtimeAclUser vmRunDir qemuMediaPrincipal "rwx")
      (runtimeLeafDir wlproxyRunDir "0770" "d2bd" "d2b")
      (runtimeAclMask wlproxyRunDir)
      (runtimeAclUser wlproxyRunDir wlproxyPrincipal "rwx")
      (runtimeAclUser wlproxyRunDir qemuMediaPrincipal "--x")
      (runtimeDefaultAclUser wlproxyRunDir qemuMediaPrincipal "rwx")
    ])
    qemuMediaVms);
in
{
  systemd.tmpfiles.rules = lib.concatLists [
    [
      "z /var/lib/d2b 0750 root d2bd -"
    ]
    (tmpfilesAcl "/var/lib/d2b" "u:microvm:--x")
    (tmpfilesAcl "/var/lib/d2b" "g:kvm:--x")
    (tmpfilesAcl "/var/lib/d2b" "g:d2b:--x")
    perVmNamedStateTraversalAcls
    # Shared runtime parents stay root-owned so broker path-safety checks can
    # create or reconcile per-VM children without trusting a daemon-writable
    # parent. The per-VM leaves below remain d2bd:d2b for daemon-owned
    # sockets and guest-control artifacts.
    (tmpfilesDir "/run/d2b/vms" "0750" "root" "d2b")
    (tmpfilesDir "/run/d2b/otel" "0750" "d2bd" "d2b")
    (tmpfilesDir "/run/d2b-gpu" "0750" "root" "d2b")
    (tmpfilesDir "/run/d2b-video" "0750" "root" "d2b")
    (tmpfilesDir "/run/d2b-wlproxy" "0750" "root" "d2b")
    perVmRuntimeTraversalAcls
    perQemuMediaRuntimeTraversalAcls
    # The traversal ACLs above add many --x named users. systemd-tmpfiles
    # recalculates the ACL mask after each a+ rule, so reassert m::rwx after
    # those entries or d2bd's named rwx ACL becomes effectively r-x and the
    # daemon cannot open /run/d2b/daemon.lock after a switch.
    (tmpfilesAcl "/run/d2b" "m::rwx")
    perNormalVmPostureTmpfiles
    perQemuMediaPostureTmpfiles
    # Reassert last: later named-user ACL grants on /run/d2b can otherwise
    # recompute the ACL mask down to r-x and clip d2bd's rwx access.
    (runtimeAclMask "/run/d2b")
  ];

  system.activationScripts.d2bGroupMigration =
    lib.stringAfter [ "users" ] ''
      target_gid="$(${pkgs.getent}/bin/getent group d2b | ${pkgs.coreutils}/bin/cut -d: -f3)"
      [ -n "$target_gid" ] || exit 0

      legacy_gids="${legacyGidsArg}"
      # Fallback: also look up live legacy groups in case the gids
      # weren't declared explicitly at eval time.
      for legacy_name in d2b-launcher d2b-launchers; do
        legacy_gid="$(${pkgs.getent}/bin/getent group "$legacy_name" | ${pkgs.coreutils}/bin/cut -d: -f3)"
        if [ -n "$legacy_gid" ] && \
           ! echo ",$legacy_gids," | ${pkgs.gnugrep}/bin/grep -q ",$legacy_gid,"; then
          legacy_gids="''${legacy_gids:+$legacy_gids,}$legacy_gid"
        fi
      done
      [ -n "$legacy_gids" ] || exit 0

      for root in /var/lib/d2b /run/d2b; do
        [ -e "$root" ] || continue
        ${groupMigrationHelper} chgrp-by-numeric-gid \
          --root "$root" \
          --legacy-gids "$legacy_gids" \
          --target-gid "$target_gid" \
          --no-follow-symlinks \
          --skip-while-lock-held /run/d2b/daemon.lock \
          ${if cfg.site.activation.failClosedOnLegacyGid
            then "--fail-closed"
            else "|| true"}
      done
    '';

  # per-sidecar-user traversal ACL on /var/lib/d2b.
  # /var/lib/d2b itself is `0750 root d2bd` (set in
  # host-daemon.nix tmpfiles); this activation script grants the
  # documented sidecar users the `--x` traversal bit on the parent
  # dir so they can reach the per-VM subdirectories owned by their
  # respective uid/gid. Without these ACLs, the 0750 parent mode
  # blocks traversal for users not in the `d2bd` group (which
  # is most sidecar users — they're per-VM-scoped and never in
  # d2bd group).
  #
  # The enumeration mirrors the user list documented in
  # `docs/reference/privileges.md` § "v1.1- state-dir ACL
  # contract". Each entry is a `--x` (execute-only / traversal)
  # grant — the sidecar user can `chdir` into the directory but
  # not read its contents; per-VM subdirectories under it have
  # their own ACLs scoped to the same sidecar user (see
  # d2bVmStatePerms above).
  #
  # Idempotent: setfacl overwrites the named entries; running
  # multiple times produces the same final ACL set.
  system.activationScripts.d2bStateDirAcl = lib.stringAfter [ "users" "d2bGroupMigration" ] (''
    set -u
    state_dir=/var/lib/d2b
    if [ ! -d "$state_dir" ]; then
      exit 0
    fi
    # Canonical root posture and static traversal ACLs are declared as
    # tmpfiles rules above. This activation script is retained only as a
    # compatibility fallback for hosts whose switch did not run tmpfiles.
    # `d2b` is the lifecycle group whose
    # members can call the public daemon socket AND read per-VM
    # SSH keys from `${cfg.site.keysDir}` (each key file is mode
    # 0640 root:d2b with a named-group ACL granting
    # read). Pre-v1.2fu58 the state-dir had no traversal grant
    # for `d2b`, so `d2b vm exec` failed
    # `stat(2)` on the key path before reaching the file — even
    # though the key existed and the operator was in the group.
    #
    # This is `--x` ONLY (chdir, no list / no read). Per-VM
    # sub-directories own their own ACLs (e.g. swtpm state, TPM
    # NVRAM, runner sockets). NO default ACL (`setfacl -d -m`)
    # is applied at this level: per-VM sub-dirs MUST keep their
    # scoped ACLs, NOT inherit a launcher-group traversal grant
    # they didn't ask for. Review confirmed the
    # default-ACL form would widen TPM-state / audit-log / runner-
    # socket surface to every launcher-group member.
    #
    # B2 / v1.2fu61 flips this to `g:d2b:--x` after the
    # group rename. See `docs/how-to/migrate-d2b-v1-1-to-v1-2.md`.
    STATE_DIR="$state_dir" \
      LAUNCHER_GROUP=d2b \
      SETFACL_BIN=${pkgs.acl}/bin/setfacl \
      . ${./host-activation.d/state-dir-acl.sh}
  '');

  # Per-graphics-VM state-dir ownership and ACLs.
  # update: ownership matches the per-VM ownership matrix
  # (`nixos-modules/options-ownership-matrix.nix`): per-VM root is
  # `d2bd:users 3770` (setgid + sticky; sticky added for issue
  # #64 so a non-owner role UID cannot rename/replace the swtpm
  # NVRAM dir). The pre-v1.1 `microvm:kvm` shape was
  # the upstream microvm.nix default; it is incompatible with the
  # daemon-native ownership matrix. Disk-image ownership is reconciled
  # by broker DiskInit immediately before cloud-hypervisor spawn; activation
  # only maintains the parent directory and inherited ACL posture here.
  system.activationScripts.d2bVmStatePerms = lib.stringAfter [ "users" "d2bGroupMigration" ]
    (lib.concatStringsSep "\n" (lib.mapAttrsToList
      (name: _: ''
        if [ -d /var/lib/d2b/vms/${name} ]; then
          ${pkgs.acl}/bin/setfacl -m "g::r-x" /var/lib/d2b/vms/${name} || true
          ${pkgs.acl}/bin/setfacl -m "u:d2b-${name}-gpu:rwx" /var/lib/d2b/vms/${name} || true
          ${pkgs.acl}/bin/setfacl -d -m "g::r-x" /var/lib/d2b/vms/${name} || true
          ${pkgs.acl}/bin/setfacl -d -m "u:d2b-${name}-gpu:rw" /var/lib/d2b/vms/${name} || true
        fi
      '')
      (lib.filterAttrs (_: vm: vm.graphics.enable) (d2bLib.normalNixosVms cfg.vms))));

  # TPM parent traversal ACLs are tmpfiles-owned. The swtpm subdir itself is
  # broker-provisioned at VM start (issue #64).
  system.activationScripts.d2bTpmStatePerms = lib.stringAfter [ "users" ]
    ''
      true
    '';

  # Non-graphics VM volume image creation/posture is broker DiskInit-owned via
  # the cloud-hypervisor node's planOps. Keep this activation snippet as an
  # empty compatibility tombstone so existing script-order references do not
  # break, but do not mutate var.img from activation.
  system.activationScripts.d2bNetVmVarImgPerms = lib.stringAfter [ "users" ]
    (lib.concatStringsSep "\n" (lib.mapAttrsToList
      (name: _: ''
          : # var.img ownership/creation is broker DiskInit-owned.
      '')
      (lib.filterAttrs (_: vm: !vm.graphics.enable) (d2bLib.normalNixosVms cfg.vms))));

  system.activationScripts.d2bMigrateOwnership = lib.stringAfter [ "users" ]
    (lib.concatStringsSep "\n" (lib.mapAttrsToList
      (name: vmCfg: ''
        # Repair orphan swtpm owners/groups in per-VM TPM state after
        # user-name renames. GNU coreutils reports unmapped owners as
        # UNKNOWN rather than the raw UID/GID, so treat any UNKNOWN
        # user/group as orphaned and repair it without disturbing
        # already-correct inodes.
        ${lib.optionalString vmCfg.tpm.enable ''
          # panel-security R4 high must-fix: orphan
          # ownership repair under runner-writable swtpm dir.
          # Previous code used `find -type d -o -type f -exec
          # bash -c 'stat $f; chown $f'`, vulnerable to swap
          # between stat and chown by a compromised swtpm
          # runner (the dir grants the swtpm UID write access
          # via d2bRoleUidAcls). Route through chown-if-
          # orphan which opens with O_NOFOLLOW + O_NONBLOCK,
          # fstats to read the current owner via fd, and only
          # fchowns when the current uid/gid don't resolve in
          # /etc/passwd / /etc/group. Also extend the running-
          # VM guard with d2bd.service for the daemon-only
          # model.
          if [ -d /var/lib/d2b/vms/${name}/swtpm ]; then
            if systemctl is-active --quiet "d2b-${name}-gpu.service" 2>/dev/null \
               || systemctl is-active --quiet "microvm@${name}.service" 2>/dev/null \
               || systemctl is-active --quiet "d2bd.service" 2>/dev/null; then
              echo "d2b: ${name} is running; skipping ownership repair"
            else
              swtpm_uid=$(${pkgs.getent}/bin/getent passwd "d2b-${name}-swtpm" | ${pkgs.coreutils}/bin/cut -d: -f3)
              swtpm_gid=$(${pkgs.getent}/bin/getent group "d2b-${name}-swtpm" | ${pkgs.coreutils}/bin/cut -d: -f3)
              if [ -n "$swtpm_uid" ] && [ -n "$swtpm_gid" ]; then
                ${pkgs.findutils}/bin/find /var/lib/d2b/vms/${name}/swtpm -mindepth 1 \
                  \( -type d -o -type f \) -print0 2>/dev/null \
                  | while IFS= read -r -d "" f; do
                  ${activationHelper} chown-if-orphan \
                    --path "$f" \
                    --uid "$swtpm_uid" \
                    --gid "$swtpm_gid" \
                    2>/dev/null || true
                done
              fi
            fi
          fi
        ''}
      '')
      (d2bLib.normalNixosVms cfg.vms)));

  # Grant the ephemeral per-role UIDs from processes.json access to
  # the per-VM state directories. v1.1.1's `stablePrincipalId` mints a unique
  # numeric UID per role from a sha256 hash of the principal
  # name; these UIDs are NOT system users but the spawned
  # runners setuid to them and need filesystem access to the
  # shares they serve (virtiofsd) or sockets they create
  # (vsock-relay, audio). Idempotent: setfacl with the same
  # entries produces the same ACL state. Runs after every
  # `nixos-rebuild switch` so a bundle update with new role
  # UIDs is automatically reflected.
  system.activationScripts.d2bRoleUidAcls = lib.stringAfter [ "users" "d2bGuestControlTokens" ] ''
    set +u
    bundle_json=/etc/d2b/processes.json
    if [ -r "$bundle_json" ]; then
      ${lib.concatStringsSep "\n" (lib.mapAttrsToList
        (name: _: ''
          guest_control_virtiofsd_uids=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | select(.id == "virtiofsd-d2b-gctl") | .profile.uid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/sort -u)
          guest_control_ch_uids=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | select(.id == "cloud-hypervisor") | .profile.uid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/sort -u)
          qemu_media_session_uids=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | select(.role == "qemu-media-runner") | .profile.uid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/sort -u)
          ${activationHelper} clear-acl-on-path --path "/var/lib/d2b/guest-control-${name}" --require-kind directory --setfacl-bin "${pkgs.acl}/bin/setfacl" 2>/dev/null || true
          ${activationHelper} clear-acl-on-path --path "/var/lib/d2b/guest-control-${name}/token" --require-kind regular --setfacl-bin "${pkgs.acl}/bin/setfacl" 2>/dev/null || true
          for uid in $qemu_media_session_uids; do
            [ "$uid" = "0" ] && continue
            ${pkgs.acl}/bin/setfacl -m "u:$uid:rwx" /var/lib/d2b/vms/${name} 2>/dev/null || true
            ${pkgs.acl}/bin/setfacl -d -m "u:$uid:rwx" /var/lib/d2b/vms/${name} 2>/dev/null || true
            ${pkgs.acl}/bin/setfacl -m "u:$uid:x" /run/d2b 2>/dev/null || true
            ${pkgs.acl}/bin/setfacl -m "u:$uid:x" /run/d2b/vms 2>/dev/null || true
            ${pkgs.acl}/bin/setfacl -m "u:$uid:rwx" /run/d2b/vms/${name} 2>/dev/null || true
            ${pkgs.acl}/bin/setfacl -d -m "u:$uid:rwx" /run/d2b/vms/${name} 2>/dev/null || true
          done
          for uid in $guest_control_virtiofsd_uids; do
            [ "$uid" = "0" ] && continue
            ${pkgs.acl}/bin/setfacl -m "u:$uid:x" /var/lib/d2b 2>/dev/null || true
            ${pkgs.acl}/bin/setfacl -m "u:$uid:x" /run/d2b 2>/dev/null || true
            ${activationHelper} setfacl-on-path \
              --path "/var/lib/d2b/guest-control-${name}" \
              --acl-spec "u:$uid:rx" \
              --also-spec "mask:r-x" \
              --require-kind directory \
              --setfacl-bin "${pkgs.acl}/bin/setfacl" \
              2>/dev/null || true
            ${activationHelper} setfacl-on-path \
              --path "/var/lib/d2b/guest-control-${name}/token" \
              --acl-spec "u:$uid:r" \
              --also-spec "mask:r--" \
              --require-kind regular \
              --setfacl-bin "${pkgs.acl}/bin/setfacl" \
              2>/dev/null || true
            d2bd_uid=$(${pkgs.getent}/bin/getent passwd d2bd | ${pkgs.coreutils}/bin/cut -d: -f3)
            d2b_gid=$(${pkgs.getent}/bin/getent group d2b | ${pkgs.coreutils}/bin/cut -d: -f3)
            if [ -n "$d2bd_uid" ] && [ -n "$d2b_gid" ]; then
              ${activationHelper} enforce-dir-posture --path /run/d2b/vms/${name} --uid "$d2bd_uid" --gid "$d2b_gid" --mode 0750 2>/dev/null || true
              ${activationHelper} enforce-dir-posture --path /run/d2b/vms/${name}/guest-control --uid "$d2bd_uid" --gid "$d2b_gid" --mode 0750 2>/dev/null || true
            fi
            ${activationHelper} clear-acl-on-path --path /run/d2b/vms/${name}/guest-control --require-kind directory --setfacl-bin "${pkgs.acl}/bin/setfacl" 2>/dev/null || true
            ${activationHelper} setfacl-on-path \
              --path "/run/d2b/vms/${name}" \
              --acl-spec "u:$uid:--x" \
              --require-kind directory \
              --setfacl-bin "${pkgs.acl}/bin/setfacl" \
              2>/dev/null || true
            ${activationHelper} setfacl-on-path \
              --path "/run/d2b/vms/${name}/guest-control" \
              --acl-spec "u:$uid:rwx" \
              --also-spec "default:u:$uid:rwx" \
              --require-kind directory \
              --setfacl-bin "${pkgs.acl}/bin/setfacl" \
              2>/dev/null || true
            for ch_uid in $guest_control_ch_uids; do
              [ "$ch_uid" = "0" ] && continue
              ${activationHelper} setfacl-on-path \
                --path "/run/d2b/vms/${name}/guest-control" \
                --acl-spec "u:$ch_uid:--x" \
                --also-spec "default:u:$ch_uid:rwX" \
                --require-kind directory \
                --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                2>/dev/null || true
            done
          done
          if [ -d /var/lib/d2b/vms/${name} ]; then
            # panel-kernel + panel-virt R3 must-fix
            # narrow /dev/kvm ACL to only KVM-consuming role UIDs, and
            # keep /dev/vhost-net narrower still. qemu-media is fd-backed:
            # it may receive broker-opened KVM/media fds, but never a
            # broad vhost-net path grant.
            # role string is top-level on the NODE (kebab-case serde
            # via #[serde(rename_all = "kebab-case")] on ProcessRole),
            # NOT on profile. Adding a new KVM-consuming role requires
            # expanding this list;
            # the eval gate tests/assertions-eval.sh has a future-
            # work item to enforce non-empty kvm_consuming_uids.
            kvm_consuming_uids=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | select(.role == "cloud-hypervisor-runner" or .role == "gpu" or .role == "qemu-media-runner") | .profile.uid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/sort -u)
            vhost_net_consuming_uids=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | select(.role == "cloud-hypervisor-runner" or .role == "gpu") | .profile.uid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/sort -u)
            video_media_uids=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | select(.role == "cloud-hypervisor-runner" or .role == "video") | .profile.uid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/sort -u)
            # Split session-socket grants by socket type:
            #   wlproxy → Wayland socket only (no PipeWire/Pulse)
            #   audio   → PipeWire/Pulse only (no Wayland)
            #   gpu/gpu-render-node → Wayland only when no proxy is emitted,
            #                         no session socket grant when proxy is active
            wlproxy_wayland_uids=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | select(.role == "wayland-proxy") | .profile.uid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/sort -u)
            audio_session_uids=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | select(.role == "audio") | .profile.uid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/sort -u)
            gpu_session_uids=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | select(.role == "gpu" or .role == "gpu-render-node") | .profile.uid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/sort -u)
            wlproxy_client_uids=$(printf '%s\n%s\n' "$gpu_session_uids" "$qemu_media_session_uids" | ${pkgs.coreutils}/bin/sort -u)
            otel_host_bridge_uids=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | select(.role == "otel-host-bridge") | .profile.uid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/sort -u)
            otel_obs_connect_uids=$(${pkgs.jq}/bin/jq -r '.vms[] | .nodes[] | select(.role == "vsock-relay" or .role == "otel-host-bridge") | .profile.uid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/sort -u)
            overlay_uid=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | .planOps[]? | select(.kind == "diskInit" and (.targetPath | endswith("/store-overlay.img"))) | .ownerUid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/head -n1)
            overlay_gid=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | .planOps[]? | select(.kind == "diskInit" and (.targetPath | endswith("/store-overlay.img"))) | .ownerGid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/head -n1)
            overlay_size_mib=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | .planOps[]? | select(.kind == "diskInit" and (.targetPath | endswith("/store-overlay.img"))) | (.sizeBytes / 1048576 | floor)' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/head -n1)
            guest_control_virtiofsd_uids=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | select(.id == "virtiofsd-d2b-gctl") | .profile.uid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/sort -u)
            if [ -n "$overlay_uid" ] && [ "$overlay_uid" != "null" ] && \
               [ -n "$overlay_gid" ] && [ "$overlay_gid" != "null" ] && \
               [ -n "$overlay_size_mib" ] && [ "$overlay_size_mib" != "null" ]; then
              # Re-assert the trusted DiskInit owner for an existing
              # store overlay. This must run once per VM, not once
              # per role UID; otherwise the last sorted role UID can
              # steal the image away from cloud-hypervisor.
              ${activationHelper} ensure-regular-file \
                --path /var/lib/d2b/vms/${name}/store-overlay.img \
                --uid "$overlay_uid" \
                --gid "$overlay_gid" \
                --mode 0600 \
                --size-mib "$overlay_size_mib" \
                2>/dev/null || true
            fi
            if [ "${name}" = "${cfg.observability.vmName}" ]; then
              obs_vsock="/var/lib/d2b/vms/${name}/vsock.sock"
              obs_state_dir="/var/lib/d2b/vms/${name}"
              if [ -S "$obs_vsock" ]; then
                for obs_uid in $otel_obs_connect_uids; do
                  [ "$obs_uid" = "0" ] && continue
                  ${pkgs.acl}/bin/setfacl -m "u:$obs_uid:x" "$obs_state_dir" 2>/dev/null || true
                  ${pkgs.acl}/bin/setfacl -d -m "u:$obs_uid:rw" "$obs_state_dir" 2>/dev/null || true
                  ${pkgs.acl}/bin/setfacl -d -m "m::rw" "$obs_state_dir" 2>/dev/null || true
                  ${pkgs.acl}/bin/setfacl -m "u:$obs_uid:rw,m::rw" "$obs_vsock" 2>/dev/null || true
                done
              fi
            fi
            for uid in $(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | .profile.uid' "$bundle_json" | ${pkgs.coreutils}/bin/sort -u); do
              [ "$uid" = "0" ] && continue
              ${pkgs.acl}/bin/setfacl -m "u:$uid:x" /var/lib/d2b 2>/dev/null || true
              # Grant traversal on both shared runtime parents so numeric
              # per-role UIDs can reach /run/d2b/vms/<vm> sockets.
              ${pkgs.acl}/bin/setfacl -m "u:$uid:x" /run/d2b 2>/dev/null || true
              ${pkgs.acl}/bin/setfacl -m "u:$uid:x" /run/d2b/vms 2>/dev/null || true
              if echo "$guest_control_virtiofsd_uids" | ${pkgs.gnugrep}/bin/grep -qx "$uid"; then
                ${activationHelper} clear-acl-on-path --path "/var/lib/d2b/guest-control-${name}" --require-kind directory --setfacl-bin "${pkgs.acl}/bin/setfacl" 2>/dev/null || true
                ${activationHelper} clear-acl-on-path --path "/var/lib/d2b/guest-control-${name}/token" --require-kind regular --setfacl-bin "${pkgs.acl}/bin/setfacl" 2>/dev/null || true
                ${activationHelper} setfacl-on-path \
                  --path "/var/lib/d2b/guest-control-${name}" \
                  --acl-spec "u:$uid:rx" \
                  --also-spec "mask:r-x" \
                  --require-kind directory \
                  --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                  2>/dev/null || true
                ${activationHelper} setfacl-on-path \
                  --path "/var/lib/d2b/guest-control-${name}/token" \
                  --acl-spec "u:$uid:r" \
                  --also-spec "mask:r--" \
                  --require-kind regular \
                  --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                  2>/dev/null || true
                d2bd_uid=$(${pkgs.getent}/bin/getent passwd d2bd | ${pkgs.coreutils}/bin/cut -d: -f3)
                d2b_gid=$(${pkgs.getent}/bin/getent group d2b | ${pkgs.coreutils}/bin/cut -d: -f3)
                if [ -n "$d2bd_uid" ] && [ -n "$d2b_gid" ]; then
                  ${activationHelper} enforce-dir-posture --path /run/d2b/vms/${name} --uid "$d2bd_uid" --gid "$d2b_gid" --mode 0750 2>/dev/null || true
                  ${activationHelper} enforce-dir-posture --path /run/d2b/vms/${name}/guest-control --uid "$d2bd_uid" --gid "$d2b_gid" --mode 0750 2>/dev/null || true
                fi
                ${activationHelper} clear-acl-on-path --path /run/d2b/vms/${name}/guest-control --require-kind directory --setfacl-bin "${pkgs.acl}/bin/setfacl" 2>/dev/null || true
                ${activationHelper} setfacl-on-path \
                  --path "/run/d2b/vms/${name}" \
                  --acl-spec "u:$uid:--x" \
                  --require-kind directory \
                  --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                  2>/dev/null || true
                ${activationHelper} setfacl-on-path \
                  --path "/run/d2b/vms/${name}/guest-control" \
                  --acl-spec "u:$uid:rwx" \
                  --also-spec "default:u:$uid:rwx" \
                  --require-kind directory \
                  --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                  2>/dev/null || true
                for ch_uid in $guest_control_ch_uids; do
                  [ "$ch_uid" = "0" ] && continue
                  ${activationHelper} setfacl-on-path \
                    --path "/run/d2b/vms/${name}/guest-control" \
                    --acl-spec "u:$ch_uid:--x" \
                    --also-spec "default:u:$ch_uid:rwX" \
                    --require-kind directory \
                    --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                    2>/dev/null || true
                done
                continue
              fi
              if echo "$otel_host_bridge_uids" | ${pkgs.gnugrep}/bin/grep -qx "$uid"; then
                ${pkgs.acl}/bin/setfacl -m "u:$uid:rwx" /run/d2b/otel 2>/dev/null || true
                ${pkgs.acl}/bin/setfacl -d -m "u:$uid:rwx" /run/d2b/otel 2>/dev/null || true
              fi
              # panel-security R2 must-fix B: /dev/kvm only for
              # KVM-consuming UIDs; /dev/vhost-net only for roles that
              # still declare a path-backed vhost-net device.
              if echo "$kvm_consuming_uids" | ${pkgs.gnugrep}/bin/grep -qx "$uid"; then
                [ -e /dev/kvm ] && ${pkgs.acl}/bin/setfacl -m "u:$uid:rw" /dev/kvm 2>/dev/null || true
              fi
              if echo "$vhost_net_consuming_uids" | ${pkgs.gnugrep}/bin/grep -qx "$uid"; then
                [ -e /dev/vhost-net ] && ${pkgs.acl}/bin/setfacl -m "u:$uid:rw" /dev/vhost-net 2>/dev/null || true
              fi
              # panel-software R2 must-fix #1, #3
              # the *.img ACL grant loop is REMOVED. New files
              # created in the VM dir inherit the per-UID rwx
              # default ACL set on lines 308-309 below
              # (setfacl -d -m). Existing pre-fu19 files keep
              # their previous ACL — operators must do one
              # `d2b vm restart <vm>` after upgrade so the
              # broker-spawned CH re-creates disk-image-shaped
              # files with the inherited ACL. Documented in the
              # migration-guide section.
              if ! echo "$otel_host_bridge_uids" | ${pkgs.gnugrep}/bin/grep -qx "$uid"; then
                ${pkgs.acl}/bin/setfacl -m "u:$uid:rwx" /var/lib/d2b/vms/${name} 2>/dev/null || true
                ${pkgs.acl}/bin/setfacl -d -m "u:$uid:rwx" /var/lib/d2b/vms/${name} 2>/dev/null || true
              fi
              # panel-security R4 critical must-fix
              # the per-subdir setfacl loop used to do
              # `[ -d "$vm_dir/$sub" ] && setfacl...` on paths
              # under the runner-writable VM root. An attacker
              # could replace `sshd-host-keys` with a symlink to
              # `/etc/ssh` between the `[ -d ]` check and the
              # setfacl call; root would then grant the runner
              # UID read ACLs on host SSH private keys. Route
              # through the fd-safe setfacl-on-path verb which
              # opens with O_NOFOLLOW (refuses symlinks) and
              # passes /proc/self/fd/<N> to setfacl so the
              # syscall operates on the inode the helper holds.
              for sub in state sshd-host-keys host-keys; do
                if ! echo "$otel_host_bridge_uids" | ${pkgs.gnugrep}/bin/grep -qx "$uid"; then
                  ${activationHelper} setfacl-on-path \
                    --path "/var/lib/d2b/vms/${name}/$sub" \
                    --acl-spec "u:$uid:rx" \
                    --also-spec "mask:r-x" \
                    --require-kind directory \
                    --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                    2>/dev/null || true
                fi
              done

              # + v1.1.2fu23
              # panel-security R4 critical must-fix: per-keyfile
              # ACL grant for ssh_host_*_key. Previously used a
              # shell glob + `[ -f ]` + setfacl, vulnerable to
              # symlink swap by the runner UID with rwx on the
              # parent. Now routes through setfacl-on-path with
              # require-kind=regular: fd-safe open, fstat
              # refusal on non-regular files, /proc/self/fd/<N>
              # for the setfacl call. The ACL grants ONLY the
              # runtime principal (per-VM scoped) and adds
              # mask:r-- to keep effective POSIX mode at 0400
              # for the ssh-host-key-preflight gate.
              if [ "$uid" != "0" ]; then
                for keyfile in /var/lib/d2b/vms/${name}/sshd-host-keys/ssh_host_*_key; do
                  [ -e "$keyfile" ] || continue
                  ${activationHelper} setfacl-on-path \
                    --path "$keyfile" \
                    --acl-spec "u:$uid:r" \
                    --also-spec "mask:r--" \
                    --require-kind regular \
                    --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                    2>/dev/null || true
                done
              fi
              # panel-security R5 medium must-fix
              # default ACL also routes through setfacl-on-path
              # (which uses openat2+RESOLVE_NO_SYMLINKS for full
              # path-component safety). The "default:u:UID:rX"
              # spec syntax is equivalent to `setfacl -d -m
              # u:UID:rX` but lets us use the fd-safe wrapper.
              for sub in store store-meta; do
                ${activationHelper} setfacl-on-path \
                  --path "/var/lib/d2b/vms/${name}/$sub" \
                  --acl-spec "u:$uid:rX" \
                  --also-spec "default:u:$uid:rX" \
                  --require-kind directory \
                  --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                  2>/dev/null || true
              done
              ${pkgs.acl}/bin/setfacl -m "u:$uid:rwx" /run/d2b/vms/${name} 2>/dev/null || true
              # DEFAULT ACL so sockets created by any
              # per-VM ephemeral UID inherit cross-UID rw. CH
              # (cloud-hypervisor uid) needs to connect to snd.sock
              # (audio uid) + gpu.sock (gpu uid) + vsock-relay sock.
              # Without default ACL, mode 0700 sockets block CH.
              ${pkgs.acl}/bin/setfacl -d -m "u:$uid:rwx" /run/d2b/vms/${name} 2>/dev/null || true
              # Option B: per-VM gpu/video runtime dirs.
              # Used as bind-mount destinations by the broker
              # (cross-domain bind for the Wayland socket, etc).
              # Mode 0750 with ACL grants for ephemeral UIDs.
              ${pkgs.acl}/bin/setfacl -m "u:$uid:rwx" /run/d2b-gpu/${name} 2>/dev/null || true
              if echo "$video_media_uids" | ${pkgs.gnugrep}/bin/grep -qx "$uid"; then
                ${pkgs.acl}/bin/setfacl -m "u:$uid:rwx" /run/d2b-video/${name} 2>/dev/null || true
                ${pkgs.acl}/bin/setfacl -d -m "u:$uid:rwx" /run/d2b-video/${name} 2>/dev/null || true
              else
                ${pkgs.acl}/bin/setfacl -x "u:$uid" /run/d2b-video/${name} 2>/dev/null || true
                ${pkgs.acl}/bin/setfacl -d -x "u:$uid" /run/d2b-video/${name} 2>/dev/null || true
              fi
              # Per-VM Wayland filter proxy runtime dir.
              # wlproxy UID gets rwx (binds the listen socket);
              # all other UIDs get --x (traverse to connect-by-path).
              if echo "$wlproxy_wayland_uids" | ${pkgs.gnugrep}/bin/grep -qx "$uid"; then
                ${pkgs.acl}/bin/setfacl -m "u:$uid:rwx" /run/d2b-wlproxy/${name} 2>/dev/null || true
                ${pkgs.acl}/bin/setfacl -d -x "u:$uid" /run/d2b-wlproxy/${name} 2>/dev/null || true
              elif [ -n "$wlproxy_wayland_uids" ] && echo "$wlproxy_client_uids" | ${pkgs.gnugrep}/bin/grep -qx "$uid"; then
                ${pkgs.acl}/bin/setfacl -m "u:$uid:--x" /run/d2b-wlproxy/${name} 2>/dev/null || true
                # DEFAULT ACL so the wlproxy-created socket (mode 0660
                # under umask 0o007) inherits a named-user rw entry for
                # the VM principal that connects to it.
                ${pkgs.acl}/bin/setfacl -d -m "u:$uid:rwx" /run/d2b-wlproxy/${name} 2>/dev/null || true
              else
                ${pkgs.acl}/bin/setfacl -m "u:$uid:--x" /run/d2b-wlproxy/${name} 2>/dev/null || true
                ${pkgs.acl}/bin/setfacl -d -x "u:$uid" /run/d2b-wlproxy/${name} 2>/dev/null || true
              fi
              # Split host session-socket grants by role:
              #   wayland-proxy role → Wayland socket only (ACL: rx on dir, rwx on wayland sock, --- on pipewire/pulse)
              #   audio role         → PipeWire/Pulse only (ACL: rx on dir, rwx on pipewire/pulse, --- on wayland)
              #   all other roles    → deny everything (--- on dir and all sockets)
              ${lib.optionalString (cfg.site.waylandUser != null) ''
                wuid=$(${pkgs.coreutils}/bin/id -u ${cfg.site.waylandUser} 2>/dev/null)
                if [ -n "$wuid" ]; then
                  rdir="/run/user/$wuid"
                  if [ -d "$rdir" ]; then
                    if echo "$wlproxy_wayland_uids" | ${pkgs.gnugrep}/bin/grep -qx "$uid"; then
                      # wlproxy: traversal on dir + rwx on Wayland socket only
                      ${activationHelper} setfacl-on-path \
                        --path "$rdir" \
                        --acl-spec "u:$uid:rx" \
                        --require-kind directory \
                        --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                        2>/dev/null || true
                      ${activationHelper} setfacl-on-path \
                        --path "$rdir/${cfg.site.waylandDisplay}" \
                        --acl-spec "u:$uid:rwx" \
                        --require-kind socket \
                        --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                        2>/dev/null || true
                      for sock in pipewire-0 pulse/native; do
                        ${activationHelper} setfacl-on-path \
                          --path "$rdir/$sock" \
                          --acl-spec "u:$uid:---" \
                          --require-kind socket \
                          --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                          2>/dev/null || true
                      done
                    elif echo "$audio_session_uids" | ${pkgs.gnugrep}/bin/grep -qx "$uid"; then
                      # audio: traversal on dir + rwx on PipeWire/Pulse, deny Wayland
                      ${activationHelper} setfacl-on-path \
                        --path "$rdir" \
                        --acl-spec "u:$uid:rx" \
                        --require-kind directory \
                        --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                        2>/dev/null || true
                      for sock in pipewire-0 pulse/native; do
                        ${activationHelper} setfacl-on-path \
                          --path "$rdir/$sock" \
                          --acl-spec "u:$uid:rwx" \
                          --require-kind socket \
                          --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                          2>/dev/null || true
                      done
                      ${activationHelper} setfacl-on-path \
                        --path "$rdir/${cfg.site.waylandDisplay}" \
                        --acl-spec "u:$uid:---" \
                        --require-kind socket \
                        --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                        2>/dev/null || true
                    elif [ -z "$wlproxy_wayland_uids" ] && echo "$gpu_session_uids" | ${pkgs.gnugrep}/bin/grep -qx "$uid"; then
                      # Direct graphics path (no wayland-proxy node): GPU gets
                      # Wayland socket access only, preserving legacy display
                      # backend behavior while keeping PipeWire/Pulse denied.
                      ${activationHelper} setfacl-on-path \
                        --path "$rdir" \
                        --acl-spec "u:$uid:rx" \
                        --require-kind directory \
                        --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                        2>/dev/null || true
                      ${activationHelper} setfacl-on-path \
                        --path "$rdir/${cfg.site.waylandDisplay}" \
                        --acl-spec "u:$uid:rwx" \
                        --require-kind socket \
                        --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                        2>/dev/null || true
                      for sock in pipewire-0 pulse/native; do
                        ${activationHelper} setfacl-on-path \
                          --path "$rdir/$sock" \
                          --acl-spec "u:$uid:---" \
                          --require-kind socket \
                          --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                          2>/dev/null || true
                      done
                    else
                      # All other roles (gpu behind the filter, video,
                      # virtiofsd, cloud-hypervisor, etc.): deny all
                      # session sockets.
                      ${activationHelper} setfacl-on-path \
                        --path "$rdir" \
                        --acl-spec "u:$uid:---" \
                        --require-kind directory \
                        --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                        2>/dev/null || true
                      for sock in pipewire-0 ${cfg.site.waylandDisplay} pulse/native; do
                        ${activationHelper} setfacl-on-path \
                          --path "$rdir/$sock" \
                          --acl-spec "u:$uid:---" \
                          --require-kind socket \
                          --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                          2>/dev/null || true
                      done
                    fi
                  fi
                fi
              ''}
            done
            # Clean up stale video-principal session ACLs even when the
            # current bundle no longer declares a video node. The UID is
            # deterministic, so disabling graphics.videoSidecar must also
            # revoke any prior Wayland/PipeWire/Pulse grants for that
            # principal without adding it to the broader role ACL loop.
            ${lib.optionalString (cfg.site.waylandUser != null) ''
              stale_video_uid="${toString (d2bLib.stablePrincipalId "d2b-${name}-video")}"
              wuid=$(${pkgs.coreutils}/bin/id -u ${cfg.site.waylandUser} 2>/dev/null)
              if [ -n "$wuid" ]; then
                rdir="/run/user/$wuid"
                if [ -d "$rdir" ]; then
                  ${activationHelper} setfacl-on-path \
                    --path "$rdir" \
                    --acl-spec "u:$stale_video_uid:---" \
                    --require-kind directory \
                    --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                    2>/dev/null || true
                  for sock in pipewire-0 ${cfg.site.waylandDisplay} pulse/native; do
                    ${activationHelper} setfacl-on-path \
                      --path "$rdir/$sock" \
                      --acl-spec "u:$stale_video_uid:---" \
                      --require-kind socket \
                      --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                      2>/dev/null || true
                  done
                fi
              fi
            ''}
            # Revoke stale qemu-media display/KVM grants whenever the VM is
            # no longer a qemu-media runtime. Keep vhost-net revoked even for
            # current qemu-media runners: fd-backed mode never exposes that
            # device path to QEMU.
            stale_qemu_media_uid="${toString (d2bLib.stablePrincipalId "d2b-${name}-qemu-media")}"
            [ -e /dev/vhost-net ] && ${pkgs.acl}/bin/setfacl -x "u:$stale_qemu_media_uid" /dev/vhost-net 2>/dev/null || true
            if ! echo "$qemu_media_session_uids" | ${pkgs.gnugrep}/bin/grep -qx "$stale_qemu_media_uid"; then
              [ -e /dev/kvm ] && ${pkgs.acl}/bin/setfacl -x "u:$stale_qemu_media_uid" /dev/kvm 2>/dev/null || true
              ${lib.optionalString (cfg.site.waylandUser != null) ''
                wuid=$(${pkgs.coreutils}/bin/id -u ${cfg.site.waylandUser} 2>/dev/null)
                if [ -n "$wuid" ]; then
                  rdir="/run/user/$wuid"
                  if [ -d "$rdir" ]; then
                    ${activationHelper} setfacl-on-path \
                      --path "$rdir" \
                      --acl-spec "u:$stale_qemu_media_uid:---" \
                      --require-kind directory \
                      --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                      2>/dev/null || true
                    for sock in pipewire-0 ${cfg.site.waylandDisplay} pulse/native; do
                      ${activationHelper} setfacl-on-path \
                        --path "$rdir/$sock" \
                        --acl-spec "u:$stale_qemu_media_uid:---" \
                        --require-kind socket \
                        --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                        2>/dev/null || true
                    done
                  fi
                fi
              ''}
            fi
            # Revoke stale direct compositor grants from the GPU principal.
            # Before this change, gpu/gpu-render-node UIDs had ACLs on the
            # real host Wayland socket. The new model routes all compositor
            # access through the wayland-proxy role; revoke any lingering
            # GPU compositor grants so the old surface is closed fail-closed.
            ${lib.optionalString (cfg.site.waylandUser != null) ''
              stale_gpu_uid="${toString (d2bLib.stablePrincipalId "d2b-${name}-gpu")}"
              wuid=$(${pkgs.coreutils}/bin/id -u ${cfg.site.waylandUser} 2>/dev/null)
              if [ -n "$wuid" ] && [ -n "$wlproxy_wayland_uids" ]; then
                rdir="/run/user/$wuid"
                if [ -d "$rdir" ]; then
                  ${activationHelper} setfacl-on-path \
                    --path "$rdir/${cfg.site.waylandDisplay}" \
                    --acl-spec "u:$stale_gpu_uid:---" \
                    --require-kind socket \
                    --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                    2>/dev/null || true
                fi
              fi
            ''}
          fi
        '')
        roleAclVms)}
    fi
    true
  '';

  #  unified-naming compatibility. Add altnames to
  # the user-visible NixOS-created bridges so the broker's
  # ApplyRoute / ApplyNftables ops (which reference derivedIfname
  # in the bundle) can find the live interface. Without altnames,
  # `ip route` against `d2b-bXXXXXXXX` returns ENODEV even though
  # `br-<env>-lan` is up.
  #
  # Reads /etc/d2b/host.json ifNameMappings to derive the
  # altname for each user-visible bridge. Tolerates missing
  # mappings (e.g. during the first activation before the bundle
  # is staged) and re-runs cleanly on every activation (altname
  # add is idempotent — exits 17 if the altname already exists).
  #  unified-naming compatibility. Add altnames to
  # the user-visible NixOS-created bridges so the broker's
  # ApplyRoute / ApplyNftables ops (which reference derivedIfname
  # in the bundle) can find the live interface. Without altnames,
  # `ip route` against `d2b-bXXXXXXXX` returns ENODEV even though
  # `br-<env>-lan` is up.
  #
  # Reads /etc/d2b/host.json ifNameMappings to derive the
  # altname for each user-visible bridge. Tolerates missing
  # mappings (e.g. during the first activation before the bundle
  # is staged) and re-runs cleanly on every activation.
  #
  # NEVER mask all
  # errors. Detect wrong-device collisions by comparing the
  # ifindex of `$user` (the user-visible bridge) and `$derived`
  # (the altname). If `$derived` already resolves to a DIFFERENT
  # interface (foreign collision), refuse and log loudly. The
  # only acceptable failure is "altname already exists on this
  # same interface" — re-add returns EEXIST in that case.
  system.activationScripts.d2bW3IfNameAltnames = lib.stringAfter [ "users" ] ''
    if [ -f /etc/d2b/host.json ] && [ -x ${pkgs.jq}/bin/jq ] && [ -x ${pkgs.iproute2}/bin/ip ]; then
      ${pkgs.jq}/bin/jq -c '.ifNameMappings // [] | .[] | select(.derivedIfname != .userVisibleName)' \
          /etc/d2b/host.json 2>/dev/null | while read -r m; do
        if [ -z "$m" ]; then continue; fi
        derived=$(printf '%s' "$m" | ${pkgs.jq}/bin/jq -r '.derivedIfname // empty')
        user=$(printf '%s' "$m" | ${pkgs.jq}/bin/jq -r '.userVisibleName // empty')
        if [ -z "$derived" ] || [ -z "$user" ]; then continue; fi
        if ! ${pkgs.iproute2}/bin/ip link show dev "$user" >/dev/null 2>&1; then
          continue
        fi
        user_idx=$(${pkgs.iproute2}/bin/ip -o link show dev "$user" 2>/dev/null | ${pkgs.gawk}/bin/awk -F': ' '{print $1}')
        if ${pkgs.iproute2}/bin/ip link show dev "$derived" >/dev/null 2>&1; then
          derived_idx=$(${pkgs.iproute2}/bin/ip -o link show dev "$derived" 2>/dev/null | ${pkgs.gawk}/bin/awk -F': ' '{print $1}')
          if [ "$user_idx" = "$derived_idx" ]; then
            : # already-bound to the same interface; nothing to do
          else
            echo "d2b: ALTNAME COLLISION: derivedIfname '$derived' resolves to ifindex $derived_idx but user-visible '$user' is ifindex $user_idx; refusing to silently add" >&2
            exit 1
          fi
        else
          # panel-software R2 must-fix #2: capture
          # `ip` stderr via command substitution instead of a
          # predictable `/tmp/d2b-altname.err` file. The
          # old approach let any local attacker pre-create
          # /tmp/d2b-altname.err as a symlink to an arbitrary
          # path; root activation would then truncate the target.
          if ! err_text=$(${pkgs.iproute2}/bin/ip link property add dev "$user" altname "$derived" 2>&1); then
            echo "d2b: failed to add altname '$derived' to '$user': $err_text" >&2
            exit 1
          fi
        fi
      done
    fi
    true
  '';

  # Enforce /run/d2b/locks ownership on every activation. The
  # tmpfiles.d rule (`d /run/d2b/locks 0700 d2bd d2bd -`,
  # in host-daemon.nix) creates the dir at boot, but broker spawn →
  # daemon restart cycles can leave it as root:d2bd which then
  # blocks the daemon's chmod(0700) idempotency call with EPERM. This
  # snippet runs on every nixos-rebuild switch + every boot and
  # idempotently re-asserts the canonical posture.
  #
  # Also enforce per-VM store-view top-level ownership (ADR 0027):
  # `store-view`, `store-view/live`, and `store-view/meta` are
  # runner/virtiofsd-readable (`d2bd:users 0755`); the broker
  # hardlink/bind ops can leave a freshly-created inode with a stricter
  # source posture that trips the ownership-matrix preflight, so
  # re-enforce here. Legacy `store`/`store-meta` are postured only when
  # present (migrated VMs). Host-only `store-view/state`,
  # `store-view/gcroots`, and `store-view/sync.lock` are NOT touched
  # here — they are broker-owned `d2bd:d2b`.
  #
  # Replace the `[ ! -L ] && chown && chmod` shell pattern with calls to
  # d2b-activation-helper which use O_DIRECTORY|O_NOFOLLOW +
  # fchown + fchmod against a held directory fd. No TOCTOU window
  # the action operates on the inode the helper opened, not on a
  # path the attacker can swap. The helper exits 2 on safety
  # refusal (path is a symlink) which we tolerate with `|| true`.
  # d2bd:d2bd uids/gids are resolved at activation time
  # via getent.
  system.activationScripts.d2bRuntimeDirPosture = lib.stringAfter [ "users" ] ''
    set +u
    d2bd_uid=$(${pkgs.getent}/bin/getent passwd d2bd | ${pkgs.coreutils}/bin/cut -d: -f3)
    d2bd_gid=$(${pkgs.getent}/bin/getent group d2bd | ${pkgs.coreutils}/bin/cut -d: -f3)
    users_gid=$(${pkgs.getent}/bin/getent group users | ${pkgs.coreutils}/bin/cut -d: -f3)
    if [ -n "$d2bd_uid" ] && [ -n "$d2bd_gid" ]; then
      if [ -d /run/d2b ]; then
        # /run/d2b must not carry default ACLs: public.sock is
        # daemon-created, and inheriting default:g::r-x makes the
        # owning d2b group read-only even after chmod 0660.
        ${pkgs.acl}/bin/setfacl -k /run/d2b 2>/dev/null || true
        ${pkgs.acl}/bin/setfacl -m "u:d2bd:rwx,g::r-x,m::rwx" /run/d2b 2>/dev/null || true
        ${activationHelper} enforce-dir-posture \
          --path /run/d2b/locks \
          --uid "$d2bd_uid" --gid "$d2bd_gid" --mode 0700 2>/dev/null || true
        ${activationHelper} enforce-dir-posture \
          --path /run/d2b/state \
          --uid "$d2bd_uid" --gid "$d2bd_gid" --mode 0700 2>/dev/null || true
      fi
      if [ -d /var/lib/d2b/vms ] && [ -n "$users_gid" ]; then
        for vm_dir in /var/lib/d2b/vms/*/; do
          vm_dir="''${vm_dir%/}"
          # ADR 0027: activation may create the missing top-level
          # store-view directory inodes and re-assert posture on them so
          # the read-only ro-store/d2b-meta virtiofsd shares always have a
          # directory to serve, but it must NOT recurse into the live
          # hardlink pool and must NOT touch broker-owned host-only state
          # (store-view/state, store-view/gcroots, store-view/sync.lock,
          # integrity leaves) — those are `d2bd:d2b` and managed
          # by the broker StoreSync path, never `users 0755`. Posture
          # only the runner-readable top-level paths here.
          for sub in store-view store-view/live store-view/meta; do
            path="$vm_dir/$sub"
            [ -d "$path" ] && ${pkgs.acl}/bin/setfacl -k "$path" 2>/dev/null || true
            ${activationHelper} enforce-dir-posture \
              --path "$path" \
              --uid "$d2bd_uid" --gid "$users_gid" --mode 0755 2>/dev/null || true
          done
          vm_name="''${vm_dir##*/}"
          live_marker="$vm_dir/store-view/live/.d2b-marker-$vm_name"
          if [ -f "$live_marker" ] && [ ! -L "$live_marker" ]; then
            ${pkgs.acl}/bin/setfacl -k "$live_marker" 2>/dev/null || true
            ${pkgs.coreutils}/bin/chown "$d2bd_uid:$users_gid" "$live_marker" 2>/dev/null || true
            ${pkgs.coreutils}/bin/chmod 0644 "$live_marker" 2>/dev/null || true
          fi
          # Legacy recovery artifacts (migrated VMs only): posture if
          # present, never created by activation.
          for sub in store store-meta; do
            path="$vm_dir/$sub"
            [ -d "$path" ] || continue
            ${pkgs.acl}/bin/setfacl -k "$path" 2>/dev/null || true
            ${activationHelper} enforce-dir-posture \
              --path "$path" \
              --uid "$d2bd_uid" --gid "$users_gid" --mode 0755 2>/dev/null || true
          done
        done
      fi
    fi
    true
  '';
}
