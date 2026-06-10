# Activation scripts for nixling.
#
# Historical one-shot repairs were removed from this module. If you're
# upgrading from a pre-public version of nixling, see CHANGELOG.md for
# the manual migration steps.
#
#   * nixlingSbctlBackup    — host-specific (maintainer's sbctl pipeline);
#                             no public framework concern. Move
#                             *-backup.tar.gz files out of $HOME by hand
#                             if you ever ran the maintainer setup.
#   * nixlingStoreChownRepair — one-shot fix for a past chown bug that
#                             leaked group=kvm into /nix/store inodes via
#                             the per-VM hardlink farm. Run the repair
#                             script from the historical /etc/nixos commit
#                             once, then forget about it. New installs are
#                             unaffected.
#   * nixlingMigrateState   — one-shot renamer (/var/lib/microvms →
#                             /var/lib/nixling/vms, plus /var/lib/swtpm
#                             → vms/<vm>/swtpm). New installs land on
#                             the new layout from the start. Existing
#                             consumers should use the migration script.
#
# What remains here
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
  nl = import ./lib.nix { inherit lib pkgs; };

  # Build the nixling-activation-helper binary (defined in
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
  cargoLock = {
    lockFile = ../packages/Cargo.lock;
    outputHashes."wl-proxy-0.1.2" = "sha256-ZKXnOZwjRkt1lbQBpAQYrYKzn6rS4gje8YWE5ek4W/E=";
  };
  activationHelperPackage = pkgs.rustPlatform.buildRustPackage {
    pname = "nixling-activation-helper";
    version = "0.0.0-bootstrap";
    src = packagesSrc;
    inherit cargoLock;
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
  groupMigrationHelperPackage = pkgs.rustPlatform.buildRustPackage {
    pname = "nixling-host-activation-helper";
    version = "0.0.0-bootstrap";
    src = packagesSrc;
    cargoBuildFlags = [ "--package" "nixling-host-activation-helper" ];
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
  groupMigrationHelper = "${groupMigrationHelperPackage}/bin/nixling-host-activation-helper";
  legacyLauncherGid = config.users.groups.nixling-launcher.gid or null;
  legacyLaunchersGid = config.users.groups.nixling-launchers.gid or null;
  legacyGidsArg = lib.concatStringsSep "," (
    builtins.filter (g: g != null)
      [ (if legacyLauncherGid == null then null else toString legacyLauncherGid)
        (if legacyLaunchersGid == null then null else toString legacyLaunchersGid)
      ]);
in
{
  system.activationScripts.nixlingGroupMigration =
    lib.stringAfter [ "users" ] ''
      target_gid="$(${pkgs.getent}/bin/getent group nixling | ${pkgs.coreutils}/bin/cut -d: -f3)"
      [ -n "$target_gid" ] || exit 0

      legacy_gids="${legacyGidsArg}"
      # Fallback: also look up live legacy groups in case the gids
      # weren't declared explicitly at eval time.
      for legacy_name in nixling-launcher nixling-launchers; do
        legacy_gid="$(${pkgs.getent}/bin/getent group "$legacy_name" | ${pkgs.coreutils}/bin/cut -d: -f3)"
        if [ -n "$legacy_gid" ] && \
           ! echo ",$legacy_gids," | ${pkgs.gnugrep}/bin/grep -q ",$legacy_gid,"; then
          legacy_gids="''${legacy_gids:+$legacy_gids,}$legacy_gid"
        fi
      done
      [ -n "$legacy_gids" ] || exit 0

      for root in /var/lib/nixling /run/nixling; do
        [ -e "$root" ] || continue
        ${groupMigrationHelper} chgrp-by-numeric-gid \
          --root "$root" \
          --legacy-gids "$legacy_gids" \
          --target-gid "$target_gid" \
          --no-follow-symlinks \
          --skip-while-lock-held /run/nixling/daemon.lock \
          ${if cfg.site.activation.failClosedOnLegacyGid
            then "--fail-closed"
            else "|| true"}
      done
    '';

  # per-sidecar-user traversal ACL on /var/lib/nixling.
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
  # `docs/reference/privileges.md` § "v1.1- state-dir ACL
  # contract". Each entry is a `--x` (execute-only / traversal)
  # grant — the sidecar user can `chdir` into the directory but
  # not read its contents; per-VM subdirectories under it have
  # their own ACLs scoped to the same sidecar user (see
  # nixlingVmStatePerms above).
  #
  # Idempotent: setfacl overwrites the named entries; running
  # multiple times produces the same final ACL set.
  system.activationScripts.nixlingStateDirAcl = lib.stringAfter [ "users" "nixlingGroupMigration" ] (''
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
    # `nixling` is the lifecycle group whose
    # members can call the public daemon socket AND read per-VM
    # SSH keys from `${cfg.site.keysDir}` (each key file is mode
    # 0640 root:nixling with a named-group ACL granting
    # read). Pre-v1.2fu58 the state-dir had no traversal grant
    # for `nixling`, so `nixling vm konsole` failed
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
    # B2 / v1.2fu61 flips this to `g:nixling:--x` after the
    # group rename. See `docs/how-to/migrate-nixling-v1-1-to-v1-2.md`.
    STATE_DIR="$state_dir" \
      LAUNCHER_GROUP=nixling \
      SETFACL_BIN=${pkgs.acl}/bin/setfacl \
      . ${./host-activation.d/state-dir-acl.sh}
    # Per-VM sidecar users: enumerated by mapAttrs over cfg.vms
    # — each VM contributes gpu/swtpm/audio/video/wlproxy users that
    # may need traversal.
  '' + lib.concatStringsSep "\n" (lib.mapAttrsToList
    (name: _: ''
      for suffix in gpu swtpm audio video wlproxy; do
        user="nixling-${name}-$suffix"
        if id "$user" >/dev/null 2>&1; then
          ${pkgs.acl}/bin/setfacl -m "u:$user:--x" /var/lib/nixling 2>/dev/null || true
        fi
      done
    '')
    cfg.vms));

  # Per-graphics-VM state dir + var.img + extra-disk ownership.
  # update: ownership matches the per-VM ownership matrix
  # (`nixos-modules/options-ownership-matrix.nix`): per-VM root is
  # `nixlingd:users 2770`. The pre-v1.1 `microvm:kvm` shape was
  # the upstream microvm.nix default; it's incompatible with the
  # daemon-native ownership matrix. var.img stays `nixlingd:kvm 0600`
  # so the cloud-
  # hypervisor runner (member of the kvm group via supplementary
  # groups) can open it. ACLs grant `nixling-<vm>-gpu` rwx on the
  # parent dir and rw on each *.img leaf so the gpu sidecar can
  # access displays.
  system.activationScripts.nixlingVmStatePerms = lib.stringAfter [ "users" "nixlingGroupMigration" ]
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
            # panel-software R2 critical must-fix
            # the old `for img in /var/lib/nixling/vms/${name}/*.img`
            # shell glob followed symlinks. The VM dir grants
            # `nixling-${name}-gpu:rwx` (line 117), so a compromised
            # gpu runner could plant `evil.img -> /etc/shadow` and
            # root would chown/chmod the target. Removed: the.img
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
  # same nixlingd:kvm 0660 as graphics VMs — the broker
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
          # panel-security R3 critical must-fix
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
          # panel-security R4 high must-fix: orphan
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
  system.activationScripts.nixlingRoleUidAcls = lib.stringAfter [ "users" ] ''
    set +u
    bundle_json=/etc/nixling/processes.json
    if [ -r "$bundle_json" ]; then
      ${lib.concatStringsSep "\n" (lib.mapAttrsToList
        (name: _: ''
          if [ -d /var/lib/nixling/vms/${name} ]; then
            # panel-kernel + panel-virt R3 must-fix
            # narrow /dev/kvm ACL to only KVM-consuming role UIDs.
            # role string is top-level on the NODE (kebab-case serde
            # via #[serde(rename_all = "kebab-case")] on ProcessRole),
            # NOT on profile. The two consumers today are
            # "cloud-hypervisor-runner" and "gpu". Adding a new
            # KVM-consuming role would require expanding this list;
            # the eval gate tests/assertions-eval.sh has a future-
            # work item to enforce non-empty kvm_consuming_uids.
            kvm_consuming_uids=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | select(.role == "cloud-hypervisor-runner" or .role == "gpu") | .profile.uid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/sort -u)
            video_media_uids=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | select(.role == "cloud-hypervisor-runner" or .role == "video") | .profile.uid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/sort -u)
            # Split session-socket grants by socket type:
            #   wlproxy → Wayland socket only (no PipeWire/Pulse)
            #   audio   → PipeWire/Pulse only (no Wayland)
            #   gpu/gpu-render-node → Wayland only when no proxy is emitted,
            #                         no session socket grant when proxy is active
            wlproxy_wayland_uids=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | select(.role == "wayland-proxy") | .profile.uid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/sort -u)
            audio_session_uids=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | select(.role == "audio") | .profile.uid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/sort -u)
            gpu_session_uids=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | select(.role == "gpu" or .role == "gpu-render-node") | .profile.uid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/sort -u)
            overlay_uid=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | .planOps[]? | select(.kind == "diskInit" and (.targetPath | endswith("/store-overlay.img"))) | .ownerUid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/head -n1)
            overlay_gid=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | .planOps[]? | select(.kind == "diskInit" and (.targetPath | endswith("/store-overlay.img"))) | .ownerGid' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/head -n1)
            overlay_size_mib=$(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | .planOps[]? | select(.kind == "diskInit" and (.targetPath | endswith("/store-overlay.img"))) | (.sizeBytes / 1048576 | floor)' "$bundle_json" 2>/dev/null | ${pkgs.coreutils}/bin/head -n1)
            if [ -n "$overlay_uid" ] && [ "$overlay_uid" != "null" ] && \
               [ -n "$overlay_gid" ] && [ "$overlay_gid" != "null" ] && \
               [ -n "$overlay_size_mib" ] && [ "$overlay_size_mib" != "null" ]; then
              # Re-assert the trusted DiskInit owner for an existing
              # store overlay. This must run once per VM, not once
              # per role UID; otherwise the last sorted role UID can
              # steal the image away from cloud-hypervisor.
              ${activationHelper} ensure-regular-file \
                --path /var/lib/nixling/vms/${name}/store-overlay.img \
                --uid "$overlay_uid" \
                --gid "$overlay_gid" \
                --mode 0600 \
                --size-mib "$overlay_size_mib" \
                2>/dev/null || true
            fi
            for uid in $(${pkgs.jq}/bin/jq -r '.vms[] | select(.vm == "${name}") | .nodes[] | .profile.uid' "$bundle_json" | ${pkgs.coreutils}/bin/sort -u); do
              [ "$uid" = "0" ] && continue
              ${pkgs.acl}/bin/setfacl -m "u:$uid:x" /var/lib/nixling 2>/dev/null || true
              # Option B: also grant traversal on
              # /run/nixling (mode 0750 nixlingd:nixling)
              # so ephemeral role UIDs (audio, video, etc.) can
              # reach the per-VM socket dir. virtiofsd skips this
              # via its own pivot_root + CAP_SYS_ADMIN; other
              # sidecars need explicit ACL.
              ${pkgs.acl}/bin/setfacl -m "u:$uid:x" /run/nixling 2>/dev/null || true
              # panel-security R2 must-fix B: /dev/kvm
              # + /dev/vhost-net only for Hypervisor/Gpu UIDs.
              if echo "$kvm_consuming_uids" | ${pkgs.gnugrep}/bin/grep -qx "$uid"; then
                [ -e /dev/kvm ] && ${pkgs.acl}/bin/setfacl -m "u:$uid:rw" /dev/kvm 2>/dev/null || true
                [ -e /dev/vhost-net ] && ${pkgs.acl}/bin/setfacl -m "u:$uid:rw" /dev/vhost-net 2>/dev/null || true
              fi
              # panel-software R2 must-fix #1, #3
              # the *.img ACL grant loop is REMOVED. New files
              # created in the VM dir inherit the per-UID rwx
              # default ACL set on lines 308-309 below
              # (setfacl -d -m). Existing pre-fu19 files keep
              # their previous ACL — operators must do one
              # `nixling vm restart <vm>` after upgrade so the
              # broker-spawned CH re-creates disk-image-shaped
              # files with the inherited ACL. Documented in the
              # migration-guide section.
              ${pkgs.acl}/bin/setfacl -m "u:$uid:rwx" /var/lib/nixling/vms/${name} 2>/dev/null || true
              ${pkgs.acl}/bin/setfacl -d -m "u:$uid:rwx" /var/lib/nixling/vms/${name} 2>/dev/null || true
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
                ${activationHelper} setfacl-on-path \
                  --path "/var/lib/nixling/vms/${name}/$sub" \
                  --acl-spec "u:$uid:rx" \
                  --also-spec "mask:r-x" \
                  --require-kind directory \
                  --setfacl-bin "${pkgs.acl}/bin/setfacl" \
                  2>/dev/null || true
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
              # panel-security R5 medium must-fix
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
              ${pkgs.coreutils}/bin/chown nixlingd:nixling /run/nixling/vms/${name} 2>/dev/null || true
              ${pkgs.coreutils}/bin/chmod 0750 /run/nixling/vms/${name} 2>/dev/null || true
              ${pkgs.acl}/bin/setfacl -m "u:$uid:rwx" /run/nixling/vms/${name} 2>/dev/null || true
              # DEFAULT ACL so sockets created by any
              # per-VM ephemeral UID inherit cross-UID rw. CH
              # (cloud-hypervisor uid) needs to connect to snd.sock
              # (audio uid) + gpu.sock (gpu uid) + vsock-relay sock.
              # Without default ACL, mode 0700 sockets block CH.
              ${pkgs.acl}/bin/setfacl -d -m "u:$uid:rwx" /run/nixling/vms/${name} 2>/dev/null || true
              # Option B: per-VM gpu/video runtime dirs.
              # Used as bind-mount destinations by the broker
              # (cross-domain bind for the Wayland socket, etc).
              # Mode 0750 with ACL grants for ephemeral UIDs.
              ${pkgs.coreutils}/bin/mkdir -p /run/nixling-gpu/${name} /run/nixling-video/${name} 2>/dev/null || true
              ${pkgs.coreutils}/bin/chown nixlingd:nixling /run/nixling-gpu/${name} /run/nixling-video/${name} 2>/dev/null || true
              ${pkgs.coreutils}/bin/chmod 0750 /run/nixling-gpu/${name} /run/nixling-video/${name} 2>/dev/null || true
              ${pkgs.acl}/bin/setfacl -m "u:$uid:rwx" /run/nixling-gpu/${name} 2>/dev/null || true
              if echo "$video_media_uids" | ${pkgs.gnugrep}/bin/grep -qx "$uid"; then
                ${pkgs.acl}/bin/setfacl -m "u:$uid:rwx" /run/nixling-video/${name} 2>/dev/null || true
                ${pkgs.acl}/bin/setfacl -d -m "u:$uid:rwx" /run/nixling-video/${name} 2>/dev/null || true
              else
                ${pkgs.acl}/bin/setfacl -x "u:$uid" /run/nixling-video/${name} 2>/dev/null || true
                ${pkgs.acl}/bin/setfacl -d -x "u:$uid" /run/nixling-video/${name} 2>/dev/null || true
              fi
              # Per-VM Wayland filter proxy runtime dir.
              # wlproxy UID gets rwx (binds the listen socket);
              # all other UIDs get --x (traverse to connect-by-path).
              ${pkgs.coreutils}/bin/mkdir -p /run/nixling-wlproxy/${name} 2>/dev/null || true
              ${pkgs.coreutils}/bin/chown nixlingd:nixling /run/nixling-wlproxy/${name} 2>/dev/null || true
              ${pkgs.coreutils}/bin/chmod 0750 /run/nixling-wlproxy/${name} 2>/dev/null || true
              if echo "$wlproxy_wayland_uids" | ${pkgs.gnugrep}/bin/grep -qx "$uid"; then
                ${pkgs.acl}/bin/setfacl -m "u:$uid:rwx" /run/nixling-wlproxy/${name} 2>/dev/null || true
                ${pkgs.acl}/bin/setfacl -d -x "u:$uid" /run/nixling-wlproxy/${name} 2>/dev/null || true
              elif [ -n "$wlproxy_wayland_uids" ] && echo "$gpu_session_uids" | ${pkgs.gnugrep}/bin/grep -qx "$uid"; then
                ${pkgs.acl}/bin/setfacl -m "u:$uid:--x" /run/nixling-wlproxy/${name} 2>/dev/null || true
                # DEFAULT ACL so the wlproxy-created socket (mode 0660
                # under umask 0o007) inherits a named-user rw entry for
                # the GPU principal that connects to it.
                ${pkgs.acl}/bin/setfacl -d -m "u:$uid:rwx" /run/nixling-wlproxy/${name} 2>/dev/null || true
              else
                ${pkgs.acl}/bin/setfacl -m "u:$uid:--x" /run/nixling-wlproxy/${name} 2>/dev/null || true
                ${pkgs.acl}/bin/setfacl -d -x "u:$uid" /run/nixling-wlproxy/${name} 2>/dev/null || true
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
              stale_video_uid="${toString (nl.stablePrincipalId "nixling-${name}-video")}"
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
            # Revoke stale direct compositor grants from the GPU principal.
            # Before this change, gpu/gpu-render-node UIDs had ACLs on the
            # real host Wayland socket. The new model routes all compositor
            # access through the wayland-proxy role; revoke any lingering
            # GPU compositor grants so the old surface is closed fail-closed.
            ${lib.optionalString (cfg.site.waylandUser != null) ''
              stale_gpu_uid="${toString (nl.stablePrincipalId "nixling-${name}-gpu")}"
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
        (lib.filterAttrs (_: vm: vm.enable) cfg.vms))}
    fi
    true
  '';

  #  unified-naming compatibility. Add altnames to
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
  #  unified-naming compatibility. Add altnames to
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
  # NEVER mask all
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
          # panel-software R2 must-fix #2: capture
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

  # Enforce /run/nixling/locks ownership on every activation. The
  # tmpfiles.d rule (`d /run/nixling/locks 0700 nixlingd nixlingd -`,
  # in host-daemon.nix) creates the dir at boot, but broker spawn →
  # daemon restart cycles can leave it as root:nixlingd which then
  # blocks the daemon's chmod(0700) idempotency call with EPERM. This
  # snippet runs on every nixos-rebuild switch + every boot and
  # idempotently re-asserts the canonical posture.
  #
  # Also enforce per-VM store-view top-level ownership (ADR 0027):
  # `store-view`, `store-view/live`, and `store-view/meta` are
  # runner/virtiofsd-readable (`nixlingd:users 0755`); the broker
  # hardlink/bind ops can leave a freshly-created inode with a stricter
  # source posture that trips the ownership-matrix preflight, so
  # re-enforce here. Legacy `store`/`store-meta` are postured only when
  # present (migrated VMs). Host-only `store-view/state`,
  # `store-view/gcroots`, and `store-view/sync.lock` are NOT touched
  # here — they are broker-owned `nixlingd:nixling`.
  #
  # Replace the `[ ! -L ] && chown && chmod` shell pattern with calls to
  # nixling-activation-helper which use O_DIRECTORY|O_NOFOLLOW +
  # fchown + fchmod against a held directory fd. No TOCTOU window
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
        # /run/nixling must not carry default ACLs: public.sock is
        # daemon-created, and inheriting default:g::r-x makes the
        # owning nixling group read-only even after chmod 0660.
        ${pkgs.acl}/bin/setfacl -k /run/nixling 2>/dev/null || true
        ${pkgs.acl}/bin/setfacl -m "g::r-x,m::r-x" /run/nixling 2>/dev/null || true
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
          # ADR 0027: activation may create the missing top-level
          # store-view directory inodes and re-assert posture on them so
          # the read-only ro-store/nl-meta virtiofsd shares always have a
          # directory to serve, but it must NOT recurse into the live
          # hardlink pool and must NOT touch broker-owned host-only state
          # (store-view/state, store-view/gcroots, store-view/sync.lock,
          # integrity leaves) — those are `nixlingd:nixling` and managed
          # by the broker StoreSync path, never `users 0755`. Posture
          # only the runner-readable top-level paths here.
          for sub in store-view store-view/live store-view/meta; do
            path="$vm_dir/$sub"
            ${pkgs.coreutils}/bin/mkdir -p "$path" 2>/dev/null || true
            [ -d "$path" ] && ${pkgs.acl}/bin/setfacl -k "$path" 2>/dev/null || true
            ${activationHelper} enforce-dir-posture \
              --path "$path" \
              --uid "$nixlingd_uid" --gid "$users_gid" --mode 0755 2>/dev/null || true
          done
          # Legacy recovery artifacts (migrated VMs only): posture if
          # present, never created by activation.
          for sub in store store-meta; do
            path="$vm_dir/$sub"
            [ -d "$path" ] || continue
            ${pkgs.acl}/bin/setfacl -k "$path" 2>/dev/null || true
            ${activationHelper} enforce-dir-posture \
              --path "$path" \
              --uid "$nixlingd_uid" --gid "$users_gid" --mode 0755 2>/dev/null || true
          done
        done
      fi
    fi
    true
  '';
}
