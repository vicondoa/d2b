# Activation scripts for nixling.
#
# Phase 2b removed the three legacy activation scripts that used to
# live here. If you're upgrading from a pre-public version of nixling
# (pre-Phase-2b), see CHANGELOG.md for the manual migration steps:
#
#   * nixlingSbctlBackup    — host-specific (maintainer's sbctl pipeline);
#                             no public framework concern. Move
#                             *-backup.tar.gz files out of $HOME by hand
#                             if you ever ran the maintainer setup.
#   * nixlingStoreChownRepair — one-shot fix for a past chown bug (P5
#                             round-1 leaked group=kvm into /nix/store
#                             inodes via the per-VM hardlink farm).
#                             If you ran a pre-Phase-2b nixling, run
#                             the repair script from the historical
#                             /etc/nixos commit once, then forget about
#                             it. New installs are unaffected.
#   * nixlingMigrateState   — one-shot renamer (/var/lib/microvms →
#                             /var/lib/nixling/vms, plus /var/lib/swtpm
#                             → vms/<vm>/swtpm). New installs land on
#                             the new layout from the start. Pre-W3
#                             consumers should use the Phase 9
#                             migration script.
#
# What remains here:
#   - nixlingVmStatePerms       — per-graphics-VM ACLs on the state dir
#                                 + var.img + every *.img disk so the
#                                 nixling-<vm>-gpu sidecar user (not
#                                 microvm) can read/write them.
#   - nixlingNetVmVarImgPerms   — net VMs use microvm:kvm 0660 on var.img.
#   - nixlingMigrateOwnership   — repair orphan swtpm-state UIDs after
#                                 service-user renames, gated on
#                                 `tpm.enable` and skipped for running VMs.
{ config, pkgs, lib, ... }:

let
  cfg = config.nixling;

  # v1.1.2fu19 panel-security R2 critical must-fix: build the
  # nixling-activation-helper binary (defined in
  # packages/nixling-host/src/bin/nixling-activation-helper.rs)
  # as a derivation here so activation scripts can call it via a
  # store path that's valid BEFORE
  # /run/current-system/sw/bin/<name> is populated (chicken-and-
  # egg during the very first activation). Each activation
  # snippet references `${activationHelper}` to get the absolute
  # store-path of the binary.
  packagesSrc = lib.cleanSourceWith {
    src = ../packages;
    filter = path: type:
      let rel = lib.removePrefix (toString ../packages + "/") (toString path);
      in !(lib.hasInfix "target" rel || lib.hasInfix ".cargo/registry" rel);
  };
  activationHelperPackage = pkgs.rustPlatform.buildRustPackage {
    pname = "nixling-activation-helper";
    version = "0.0.0-bootstrap";
    src = packagesSrc;
    cargoLock.lockFile = ../packages/Cargo.lock;
    cargoBuildFlags = [ "--package" "nixling-host" "--bin" "nixling-activation-helper" ];
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
      install -Dm755 target/x86_64-unknown-linux-gnu/release/nixling-activation-helper $out/bin/nixling-activation-helper 2>/dev/null \
        || install -Dm755 target/release/nixling-activation-helper $out/bin/nixling-activation-helper
      runHook postInstall
    '';
  };
  activationHelper = "${activationHelperPackage}/bin/nixling-activation-helper";
in
{
  # v1.1-P5: per-sidecar-user traversal ACL on /var/lib/nixling.
  # /var/lib/nixling itself is `0750 root nixlingd` (set in
  # host-daemon.nix tmpfiles); this activation script grants the
  # documented sidecar users the `--x` traversal bit on the parent
  # dir so they can reach the per-VM subdirectories owned by their
  # respective uid/gid. Without these ACLs, the 0750 parent mode
  # blocks traversal for users not in the `nixlingd` group (which
  # is most sidecar users — they're per-VM-scoped and never in
  # nixlingd group).
  #
  # The enumeration mirrors the user list documented in
  # `docs/reference/privileges.md` § "v1.1-P5 state-dir ACL
  # contract". Each entry is a `--x` (execute-only / traversal)
  # grant — the sidecar user can `chdir` into the directory but
  # not read its contents; per-VM subdirectories under it have
  # their own ACLs scoped to the same sidecar user (see
  # nixlingVmStatePerms above).
  #
  # Idempotent: setfacl overwrites the named entries; running
  # multiple times produces the same final ACL set.
  system.activationScripts.nixlingStateDirAcl = lib.stringAfter [ "users" ] (''
    set -u
    state_dir=/var/lib/nixling
    if [ ! -d "$state_dir" ]; then
      exit 0
    fi
    # Re-assert canonical mode + ownership (defense-in-depth
    # against any 0755 chmod workaround a previous v0.x install
    # may have left behind).
    chown root "$state_dir" || true
    chgrp nixlingd "$state_dir" || true
    chmod 0750 "$state_dir" || true
    # Per-sidecar-user traversal grants. Each grant is `--x`
    # (chdir only); the per-VM subdir under it owns its own
    # read/write ACL.
    # `microvm` is a system user (created by host.nix /
    # nixos.users); grant `u:microvm:--x`.
    ${pkgs.acl}/bin/setfacl -m "u:microvm:--x" "$state_dir" 2>/dev/null || true
    # `kvm` is a Linux GROUP (not a user) — every sidecar that
    # opens /dev/kvm is in this group. Grant `g:kvm:--x` so any
    # group member can traverse the state-dir parent. Treating
    # `kvm` as a user (the v1.1-rc1 bug) silently grants nothing
    # because no `kvm` user exists.
    ${pkgs.acl}/bin/setfacl -m "g:kvm:--x" "$state_dir" 2>/dev/null || true
    # Per-VM sidecar users: enumerated by mapAttrs over cfg.vms
    # — each VM contributes gpu/swtpm/audio/video users that
    # may need traversal.
  '' + lib.concatStringsSep "\n" (lib.mapAttrsToList
    (name: _: ''
      for suffix in gpu swtpm audio video; do
        user="nixling-${name}-$suffix"
        if id "$user" >/dev/null 2>&1; then
          ${pkgs.acl}/bin/setfacl -m "u:$user:--x" /var/lib/nixling 2>/dev/null || true
        fi
      done
    '')
    cfg.vms));

  # Per-graphics-VM state dir + var.img + extra-disk ownership.
  # v1.1.1 update: ownership matches the per-VM ownership matrix
  # (`nixos-modules/options-ownership-matrix.nix`): per-VM root is
  # `nixlingd:users 2770`. The pre-v1.1 `microvm:kvm` shape was
  # the upstream microvm.nix default; it's incompatible with the
  # daemon-native ownership matrix that ph2-p2-ownership-matrix
  # enforces. var.img stays `nixlingd:kvm 0600` so the cloud-
  # hypervisor runner (member of the kvm group via supplementary
  # groups) can open it. ACLs grant `nixling-<vm>-gpu` rwx on the
  # parent dir and rw on each *.img leaf so the gpu sidecar can
  # access displays.
  system.activationScripts.nixlingVmStatePerms = lib.stringAfter [ "users" ]
    (lib.concatStringsSep "\n" (lib.mapAttrsToList
      (name: _: ''
        if [ -d /var/lib/nixling/vms/${name} ]; then
          chown nixlingd /var/lib/nixling/vms/${name} || true
          chgrp users /var/lib/nixling/vms/${name} || true
          chmod 2770 /var/lib/nixling/vms/${name} || true
          ${pkgs.acl}/bin/setfacl -m "g::r-x" /var/lib/nixling/vms/${name} || true
          ${pkgs.acl}/bin/setfacl -m "u:nixling-${name}-gpu:rwx" /var/lib/nixling/vms/${name} || true
          ${pkgs.acl}/bin/setfacl -d -m "g::r-x" /var/lib/nixling/vms/${name} || true
          ${pkgs.acl}/bin/setfacl -d -m "u:nixling-${name}-gpu:rw" /var/lib/nixling/vms/${name} || true
          if systemctl is-active --quiet "nixling-${name}-gpu.service" 2>/dev/null; then
            echo "nixling: ${name} is running; skipping disk image ownership fix (apply on next nixling vm restart)"
          else
            # v1.1.2fu19 panel-software R2 critical must-fix:
            # the old `for img in /var/lib/nixling/vms/${name}/*.img`
            # shell glob followed symlinks. The VM dir grants
            # `nixling-${name}-gpu:rwx` (line 117), so a compromised
            # gpu runner could plant `evil.img -> /etc/shadow` and
            # root would chown/chmod the target. Removed: the .img
            # ACL grants now flow exclusively through the
            # nixlingRoleUidAcls activation script below, which is
            # also being hardened. Existing files keep their
            # previous ACL; new files inherit it via the parent
            # dir's default ACL (setfacl -d -m at line 119).
            #
            # Operators upgrading from v1.1.1: do one
            # `nixling vm restart <vm>` after `nixos-rebuild switch`
            # to ensure the broker-spawned CH re-applies ACLs to
            # any new disk images it creates.
            :
          fi
        fi
      '')
      (lib.filterAttrs (_: vm: vm.enable && vm.graphics.enable) cfg.vms)));

  # TPM VMs need an explicit traverse ACL for the dedicated swtpm user on
  # the parent VM state dir, regardless of whether the VM is graphics-backed
  # or headless. The swtpm StateDirectory itself is 0700-owned by the swtpm
  # user; this parent-dir grant is only for the path walk into `swtpm/`.
  system.activationScripts.nixlingTpmStatePerms = lib.stringAfter [ "users" ]
    (lib.concatStringsSep "\n" (lib.mapAttrsToList
      (name: _: ''
        if [ -d /var/lib/nixling/vms/${name} ]; then
          # v0.1.4 fix: nixling-${name}-swtpm needs +x on the parent
          # state dir to traverse into its `swtpm/` subdir (where
          # systemd's StateDirectory= places it). Without this grant
          # the swtpm service starts but fails to open
          # tpm2-00.permall with EACCES, libtpms enters failure mode,
          # and the VM boots with a freshly-initialised TPM —
          # triggering Entra/Intune device-tampering alerts for
          # tenant-enrolled VMs.
          ${pkgs.acl}/bin/setfacl -m "u:nixling-${name}-swtpm:--x" /var/lib/nixling/vms/${name} || true
        fi
      '')
      (lib.filterAttrs (_: vm: vm.enable && vm.tpm.enable) cfg.vms)));

  # Non-graphics VMs (net VMs) also need correct ownership on var.img.
  # v1.1.1: same nixlingd:kvm 0660 as graphics VMs — the broker
  # spawns CH as nixlingd (member of kvm via supplementary groups)
  # and opens var.img read/write.
  system.activationScripts.nixlingNetVmVarImgPerms = lib.stringAfter [ "users" ]
    (lib.concatStringsSep "\n" (lib.mapAttrsToList
      (name: _: ''
          if [ -d /var/lib/nixling/vms/${name} ]; then
            chown nixlingd /var/lib/nixling/vms/${name} || true
            chgrp users /var/lib/nixling/vms/${name} || true
            chmod 2770 /var/lib/nixling/vms/${name} || true
          fi
          # v1.1.2fu20 panel-security R3 critical must-fix:
          # var.img repair must NOT use `[ -f ]` + chown/chmod
          # because the runner-UID has rwx on the parent dir
          # (granted by nixlingRoleUidAcls below). An attacker
          # could swap var.img for a symlink between the check
          # and the chown. Route through ensure-regular-file
          # which uses O_NOFOLLOW + fchown + fchmod against a
          # held fd. The size-mib=0 special-cases re-asserting
          # ownership/mode on an existing file: ensure-regular-
          # file's `Err(AlreadyExists)` branch never re-truncates,
          # it only re-fchown+re-fchmod.
          if systemctl is-active --quiet "nixling-${name}-runner.service" 2>/dev/null \
             || systemctl is-active --quiet "nixlingd.service" 2>/dev/null; then
            : # daemon-only model: skip var.img repair while live VM may be running
          else
            kvm_gid=$(${pkgs.getent}/bin/getent group kvm | ${pkgs.coreutils}/bin/cut -d: -f3)
            nixlingd_uid=$(${pkgs.getent}/bin/getent passwd nixlingd | ${pkgs.coreutils}/bin/cut -d: -f3)
            if [ -n "$kvm_gid" ] && [ -n "$nixlingd_uid" ]; then
              ${activationHelper} ensure-regular-file \
                --path /var/lib/nixling/vms/${name}/var.img \
                --uid "$nixlingd_uid" --gid "$kvm_gid" --mode 0660 --size-mib 0 \
                2>/dev/null || true
            fi
          fi
      '')
      (lib.filterAttrs (_: vm: vm.enable && !vm.graphics.enable) cfg.vms)));

  system.activationScripts.nixlingMigrateOwnership = lib.stringAfter [ "users" ]
    (lib.concatStringsSep "\n" (lib.mapAttrsToList
      (name: vmCfg: ''
        # Repair orphan swtpm owners/groups in per-VM TPM state after
        # user-name renames. GNU coreutils reports unmapped owners as
        # UNKNOWN rather than the raw UID/GID, so treat any UNKNOWN
        # user/group as orphaned and repair it without disturbing
        # already-correct inodes.
        ${lib.optionalString vmCfg.tpm.enable ''
          # v1.1.2fu23 panel-security R4 high must-fix: orphan
          # ownership repair under runner-writable swtpm dir.
          # Previous code used `find -type d -o -type f -exec
          # bash -c 'stat $f; chown $f'`, vulnerable to swap
          # between stat and chown by a compromised swtpm
          # runner (the dir grants the swtpm UID write access
          # via nixlingRoleUidAcls). Route through chown-if-
          # orphan which opens with O_NOFOLLOW + O_NONBLOCK,
          # fstats to read the current owner via fd, and only
          # fchowns when the current uid/gid don't resolve in
          # /etc/passwd / /etc/group. Also extend the running-
          # VM guard with nixlingd.service for the daemon-only
          # model.
          if [ -d /var/lib/nixling/vms/${name}/swtpm ]; then
            if systemctl is-active --quiet "nixling-${name}-gpu.service" 2>/dev/null \
               || systemctl is-active --quiet "microvm@${name}.service" 2>/dev/null \
               || systemctl is-active --quiet "nixlingd.service" 2>/dev/null; then
              echo "nixling: ${name} is running; skipping ownership repair"
            else
              swtpm_uid=$(${pkgs.getent}/bin/getent passwd "nixling-${name}-swtpm" | ${pkgs.coreutils}/bin/cut -d: -f3)
              swtpm_gid=$(${pkgs.getent}/bin/getent group "nixling-${name}-swtpm" | ${pkgs.coreutils}/bin/cut -d: -f3)
              if [ -n "$swtpm_uid" ] && [ -n "$swtpm_gid" ]; then
                ${pkgs.findutils}/bin/find /var/lib/nixling/vms/${name}/swtpm -mindepth 1 \
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
      (lib.filterAttrs (_: vm: vm.enable) cfg.vms)));

  # v1.1.1 live-deploy fu10: Grant the ephemeral per-role
  # UIDs from processes.json access to the per-VM state
  # directories. v1.1.1's `stablePrincipalId` mints a unique
  # numeric UID per role from a sha256 hash of the principal
  # name; these UIDs are NOT system users but the spawned
  # runners setuid() to them and need filesystem access to the
  # shares they serve (virtiofsd) or sockets they create
  # (vsock-relay, audio). Idempotent: setfacl with the same
  # entries produces the same ACL state. Runs after every
  # `nixos-rebuild switch` so a bundle update with new role
  # UIDs is automatically reflected.
  system.activationScripts.nixlingRoleUidAcls = lib.stringAfter [ "users" ] ''
    set +u
    bundle_json=/etc/nixling/processes.json
    if [ -r "$bundle_json" ]; then
      ${lib.concatStringsSep "\n" (lib.mapAttrsToList
        (name: _: ''
          if [ -d /var/lib/nixling/vms/${name} ]; then
            # v1.1.2fu20 panel-kernel + panel-virt R3 must-fix:
            # narrow /dev/kvm ACL to only KVM-consuming role UIDs.
            # role string is top-level on the NODE (kebab-case serde
            # via #[serde(rename_all = "kebab-case")] on ProcessRole),
            # NOT on profile. The two consumers today are
            # "cloud-hypervisor-runner" and "gpu". Adding a new
            # KVM-consuming role would require expanding this list;
            # the eval gate tests/assertions-eval.sh has a future-
            # work item to enforce non-empty kvm_consuming_uids.
            kvm_consuming_uids=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | select(.role == "cloud-hypervisor-runner" or .role == "gpu") | .profile.uid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/sort -u)
            for uid in $(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | .profile.uid' "$bundle_json" | ${pkgs.coreutils}/bin/sort -u); do
              [ "$uid" = "0" ] && continue
              ${pkgs.acl}/bin/setfacl -m "u:$uid:x" /var/lib/nixling 2>/dev/null || true
              # v1.1.1fu11 Option B: also grant traversal on
              # /run/nixling (mode 0750 nixlingd:nixling-launchers)
              # so ephemeral role UIDs (audio, video, etc.) can
              # reach the per-VM socket dir. virtiofsd skips this
              # via its own pivot_root + CAP_SYS_ADMIN; other
              # sidecars need explicit ACL.
              ${pkgs.acl}/bin/setfacl -m "u:$uid:x" /run/nixling 2>/dev/null || true
              # v1.1.2fu19 panel-security R2 must-fix B: /dev/kvm
              # + /dev/vhost-net only for Hypervisor/Gpu UIDs.
              if echo "$kvm_consuming_uids" | ${pkgs.gnugrep}/bin/grep -qx "$uid"; then
                [ -e /dev/kvm ] && ${pkgs.acl}/bin/setfacl -m "u:$uid:rw" /dev/kvm 2>/dev/null || true
                [ -e /dev/vhost-net ] && ${pkgs.acl}/bin/setfacl -m "u:$uid:rw" /dev/vhost-net 2>/dev/null || true
              fi
              # v1.1.2fu19 panel-security R2 critical must-fix:
              # delegate store-overlay.img creation to the
              # nixling-activation-helper Rust binary which uses
              # O_CREAT|O_EXCL|O_NOFOLLOW + ftruncate + fchown +
              # fchmod against a held fd. No TOCTOU window: the
              # action operates on the inode the helper opened,
              # not on a path the attacker can swap. The helper
              # exits 2 on safety refusal (symlink at target)
              # which we tolerate with `|| true` — activation
              # continues; operator sees the refusal in stderr.
              overlay=/var/lib/nixling/vms/${name}/store-overlay.img
              ${activationHelper} ensure-regular-file \
                --path "$overlay" \
                --uid "$uid" \
                --gid "$uid" \
                --mode 0600 \
                --size-mib 2048 \
                2>/dev/null || true
              # v1.1.2fu19 panel-software R2 must-fix #1, #3:
              # the *.img ACL grant loop is REMOVED. New files
              # created in the VM dir inherit the per-UID rwx
              # default ACL set on lines 308-309 below
              # (setfacl -d -m). Existing pre-fu19 files keep
              # their previous ACL — operators must do one
              # `nixling vm restart <vm>` after upgrade so the
              # broker-spawned CH re-creates disk-image-shaped
              # files with the inherited ACL. Documented in the
              # v1.1.2 migration-guide section.
              ${pkgs.acl}/bin/setfacl -m "u:$uid:rwx" /var/lib/nixling/vms/${name} 2>/dev/null || true
              ${pkgs.acl}/bin/setfacl -d -m "u:$uid:rwx" /var/lib/nixling/vms/${name} 2>/dev/null || true
              # v1.1.2fu23 panel-security R4 critical must-fix:
              # the per-subdir setfacl loop used to do
              # `[ -d "$vm_dir/$sub" ] && setfacl ...` on paths
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
                ${activationHelper} setfacl-on-path \
                  --path "/var/lib/nixling/vms/${name}/$sub" \
                  --acl-spec "u:$uid:rx" \
                  --also-spec "mask:r-x" \
                  --require-kind directory \
                  --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                  2>/dev/null || true
              done

              # v1.1.1fu15 (panel-virt must-fix #2) + v1.1.2fu23
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
                for keyfile in /var/lib/nixling/vms/${name}/sshd-host-keys/ssh_host_*_key; do
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
              # v1.1.2fu24 panel-security R5 medium must-fix:
              # default ACL also routes through setfacl-on-path
              # (which uses openat2+RESOLVE_NO_SYMLINKS for full
              # path-component safety). The "default:u:UID:rX"
              # spec syntax is equivalent to `setfacl -d -m
              # u:UID:rX` but lets us use the fd-safe wrapper.
              for sub in store store-meta; do
                ${activationHelper} setfacl-on-path \
                  --path "/var/lib/nixling/vms/${name}/$sub" \
                  --acl-spec "u:$uid:rX" \
                  --also-spec "default:u:$uid:rX" \
                  --require-kind directory \
                  --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                  2>/dev/null || true
              done
              ${pkgs.coreutils}/bin/mkdir -p /run/nixling/vms/${name} 2>/dev/null || true
              ${pkgs.coreutils}/bin/chown nixlingd:nixling-launcher /run/nixling/vms/${name} 2>/dev/null || true
              ${pkgs.coreutils}/bin/chmod 0750 /run/nixling/vms/${name} 2>/dev/null || true
              ${pkgs.acl}/bin/setfacl -m "u:$uid:rwx" /run/nixling/vms/${name} 2>/dev/null || true
              # v1.1.1fu13k: DEFAULT ACL so sockets created by any
              # per-VM ephemeral UID inherit cross-UID rw. CH
              # (cloud-hypervisor uid) needs to connect to snd.sock
              # (audio uid) + gpu.sock (gpu uid) + vsock-relay sock.
              # Without default ACL, mode 0700 sockets block CH.
              ${pkgs.acl}/bin/setfacl -d -m "u:$uid:rwx" /run/nixling/vms/${name} 2>/dev/null || true
              # v1.1.1fu11 Option B: per-VM gpu/video runtime dirs.
              # Used as bind-mount destinations by the broker
              # (cross-domain bind for the Wayland socket, etc).
              # Mode 0750 with ACL grants for ephemeral UIDs.
              ${pkgs.coreutils}/bin/mkdir -p /run/nixling-gpu/${name} /run/nixling-video/${name} 2>/dev/null || true
              ${pkgs.coreutils}/bin/chown nixlingd:nixling-launcher /run/nixling-gpu/${name} /run/nixling-video/${name} 2>/dev/null || true
              ${pkgs.coreutils}/bin/chmod 0750 /run/nixling-gpu/${name} /run/nixling-video/${name} 2>/dev/null || true
              ${pkgs.acl}/bin/setfacl -m "u:$uid:rwx" /run/nixling-gpu/${name} 2>/dev/null || true
              ${pkgs.acl}/bin/setfacl -m "u:$uid:rwx" /run/nixling-video/${name} 2>/dev/null || true
              # Pre-create the Wayland socket bind destination as
              # an empty regular file (mount --bind needs the dst
              # to exist with the same inode-type as src).
              [ ! -e /run/nixling-gpu/${name}/wayland-0 ] && ${pkgs.coreutils}/bin/touch /run/nixling-gpu/${name}/wayland-0 2>/dev/null || true
              # v1.1.1fu11 Option B: grant ephemeral role UIDs
              # access to the Wayland user's PipeWire + Wayland
              # sockets. This is the "BindSessionSocket" mechanism
              # done declaratively at activation time: the broker's
              # SpawnRunner setuids the runner to the ephemeral
              # UID; the role then connects to PipeWire/Wayland as
              # that UID, which needs explicit setfacl since the
              # ephemeral UID has no group membership.
              ${lib.optionalString (cfg.site.waylandUser != null) ''
                wuid=$(${pkgs.coreutils}/bin/id -u ${cfg.site.waylandUser} 2>/dev/null)
                if [ -n "$wuid" ]; then
                  rdir="/run/user/$wuid"
                  if [ -d "$rdir" ]; then
                    ${pkgs.acl}/bin/setfacl -m "u:$uid:rx" "$rdir" 2>/dev/null || true
                    for sock in pipewire-0 wayland-0 pulse/native; do
                      [ -e "$rdir/$sock" ] && ${pkgs.acl}/bin/setfacl -m "u:$uid:rwx" "$rdir/$sock" 2>/dev/null || true
                    done
                  fi
                fi
              ''}
            done
          fi
        '')
        (lib.filterAttrs (_: vm: vm.enable) cfg.vms))}
    fi
    true
  '';

  # v1.1.1fu12d: W3 unified-naming compatibility. Add altnames to
  # the user-visible NixOS-created bridges so the broker's
  # ApplyRoute / ApplyNftables ops (which reference derivedIfname
  # in the bundle) can find the live interface. Without altnames,
  # `ip route` against `nl-bXXXXXXXX` returns ENODEV even though
  # `br-<env>-lan` is up.
  #
  # Reads /etc/nixling/host.json ifNameMappings to derive the
  # altname for each user-visible bridge. Tolerates missing
  # mappings (e.g. during the first activation before the bundle
  # is staged) and re-runs cleanly on every activation (altname
  # add is idempotent — exits 17 if the altname already exists).
  # v1.1.1fu14d + fu15: W3 unified-naming compatibility. Add altnames to
  # the user-visible NixOS-created bridges so the broker's
  # ApplyRoute / ApplyNftables ops (which reference derivedIfname
  # in the bundle) can find the live interface. Without altnames,
  # `ip route` against `nl-bXXXXXXXX` returns ENODEV even though
  # `br-<env>-lan` is up.
  #
  # Reads /etc/nixling/host.json ifNameMappings to derive the
  # altname for each user-visible bridge. Tolerates missing
  # mappings (e.g. during the first activation before the bundle
  # is staged) and re-runs cleanly on every activation.
  #
  # v1.1.1fu15 (panel-networking must-fix #1): NEVER mask all
  # errors. Detect wrong-device collisions by comparing the
  # ifindex of `$user` (the user-visible bridge) and `$derived`
  # (the altname). If `$derived` already resolves to a DIFFERENT
  # interface (foreign collision), refuse and log loudly. The
  # only acceptable failure is "altname already exists on this
  # same interface" — re-add returns EEXIST in that case.
  system.activationScripts.nixlingW3IfNameAltnames = lib.stringAfter [ "users" ] ''
    if [ -f /etc/nixling/host.json ] && [ -x ${pkgs.jq}/bin/jq ] && [ -x ${pkgs.iproute2}/bin/ip ]; then
      ${pkgs.jq}/bin/jq -c '.ifNameMappings // [] | .[] | select(.derivedIfname != .userVisibleName)' \
          /etc/nixling/host.json 2>/dev/null | while read -r m; do
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
            echo "nixling: ALTNAME COLLISION: derivedIfname '$derived' resolves to ifindex $derived_idx but user-visible '$user' is ifindex $user_idx; refusing to silently add" >&2
            exit 1
          fi
        else
          # v1.1.2fu19 panel-software R2 must-fix #2: capture
          # `ip` stderr via command substitution instead of a
          # predictable `/tmp/nixling-altname.err` file. The
          # old approach let any local attacker pre-create
          # /tmp/nixling-altname.err as a symlink to an arbitrary
          # path; root activation would then truncate the target.
          if ! err_text=$(${pkgs.iproute2}/bin/ip link property add dev "$user" altname "$derived" 2>&1); then
            echo "nixling: failed to add altname '$derived' to '$user': $err_text" >&2
            exit 1
          fi
        fi
      done
    fi
    true
  '';

  # v1.1.1fu14 B1 + B2: enforce /run/nixling/locks ownership on
  # every activation. The tmpfiles.d rule (`d /run/nixling/locks
  # 0700 nixlingd nixlingd -`, in host-daemon.nix) creates the dir
  # at boot, but live-deploy iteration (broker spawn → daemon
  # restart cycles) can leave it as root:nixlingd which then
  # blocks the daemon's chmod(0700) idempotency call with EPERM.
  # This snippet runs on every nixos-rebuild switch + every boot
  # and idempotently re-asserts the canonical posture.
  #
  # Companion B4: enforce per-VM store / store-meta ownership
  # (`nixlingd:users 2775`); the BindMountFromHardlinkFarm broker
  # op preserves the source's root:kvm 2755 which trips the
  # ownership-matrix preflight. Re-enforce here.
  #
  # v1.1.2fu19 panel-security R2 critical must-fix: replace the
  # `[ ! -L ] && chown && chmod` shell pattern with calls to
  # nixling-activation-helper which use O_DIRECTORY|O_NOFOLLOW +
  # fchown + fchmod against a held directory fd. No TOCTOU window:
  # the action operates on the inode the helper opened, not on a
  # path the attacker can swap. The helper exits 2 on safety
  # refusal (path is a symlink) which we tolerate with `|| true`.
  # nixlingd:nixlingd uids/gids are resolved at activation time
  # via getent.
  system.activationScripts.nixlingRuntimeDirPosture = lib.stringAfter [ "users" ] ''
    set +u
    nixlingd_uid=$(${pkgs.getent}/bin/getent passwd nixlingd | ${pkgs.coreutils}/bin/cut -d: -f3)
    nixlingd_gid=$(${pkgs.getent}/bin/getent group nixlingd | ${pkgs.coreutils}/bin/cut -d: -f3)
    users_gid=$(${pkgs.getent}/bin/getent group users | ${pkgs.coreutils}/bin/cut -d: -f3)
    if [ -n "$nixlingd_uid" ] && [ -n "$nixlingd_gid" ]; then
      if [ -d /run/nixling ]; then
        ${activationHelper} enforce-dir-posture \
          --path /run/nixling/locks \
          --uid "$nixlingd_uid" --gid "$nixlingd_gid" --mode 0700 2>/dev/null || true
        ${activationHelper} enforce-dir-posture \
          --path /run/nixling/state \
          --uid "$nixlingd_uid" --gid "$nixlingd_gid" --mode 0700 2>/dev/null || true
      fi
      if [ -d /var/lib/nixling/vms ] && [ -n "$users_gid" ]; then
        for vm_dir in /var/lib/nixling/vms/*/; do
          vm_dir="''${vm_dir%/}"
          for sub in store store-meta; do
            ${activationHelper} enforce-dir-posture \
              --path "$vm_dir/$sub" \
              --uid "$nixlingd_uid" --gid "$users_gid" --mode 2775 2>/dev/null || true
          done
        done
      fi
    fi
    true
  '';
}