# Nixling-managed SSH keys.
#
# This module owns two halves of the same flow:
#
#  1. Host-side activation (`nixlingGenerateKeys`):
#     For each enabled VM declared via `nixling.vms.<name>` (workload
#     OR net), generate `<keysDir>/<vm>_ed25519` + matching pubkey on
#     first activation if missing. Subsequent activations only repair
#     modes / ACLs. Atomic install via a staging tempfile and `mv -T`;
#     the entire generation step runs under flock on
#     `<keysDir>/.lock` so two concurrent `nixos-rebuild switch` runs
#     can't race on key creation.
#
#  2. Per-VM host-keys staging dir
#     (`<stateDir>/vms/<vm>/host-keys/`):
#     The activation script also stages two files there per VM —
#     `host.pub` (the framework-managed pubkey) and
#     `user-authorized-keys` (the resolved content of
#     `cfg.site.userAuthorizedKeys` and
#     `cfg.vms.<vm>.userAuthorizedKeys`). host.nix mounts this dir
#     into the guest at /run/nixling-host-keys/ via a virtiofs share
#     (see ./host.nix for the share declaration).
#
# The matching guest-side consumer (`nixling-load-host-keys.service`,
# declared in ./base.nix) reads /run/nixling-host-keys/ at boot and
# writes the union of host.pub + user-authorized-keys into the SSH
# user's ~/.ssh/authorized_keys.
#
# Operator-visible side effects (none under normal operation):
#   - `<keysDir>/<vm>_ed25519`        owner root:nixling mode 0640.
#                                     The CLI copies to a tempfile (mode 0600,
#                                     caller-owned) before passing to ssh.
#   - `<keysDir>/<vm>_ed25519.pub`    owner root:root mode 0644.
#   - `<stateDir>/vms/<vm>/host-keys/host.pub`
#   - `<stateDir>/vms/<vm>/host-keys/user-authorized-keys`
{ config, pkgs, lib, ... }:

let
  cfg = config.nixling;
  nl = import ./lib.nix { inherit lib; };
  normalNixosVms = nl.normalNixosVms cfg.vms;

  # Resolve a `userAuthorizedKeys` entry (path | string) to its raw
  # text. Paths get read into the Nix store at eval time and surfaced
  # as a string, then concatenated. Eval-time validation in
  # assertions.nix already guards against private-key markers and
  # unsupported pubkey types, so by the time we get here we trust the
  # contents.
  readKey = entry:
    if builtins.isPath entry || lib.isStorePath entry
    then builtins.readFile entry
    else toString entry;

  # Per-VM merged userAuthorizedKeys content (site + VM, deduped at
  # write time on the host).
  perVmUserAuthorizedKeysText = vm:
    lib.concatStringsSep "\n"
      (map readKey (cfg.site.userAuthorizedKeys ++ vm.userAuthorizedKeys));

  # Generate a single shell block for one VM that ensures its key
  # pair exists, repairs perms, and stages the host-keys share dir.
  perVmGenScript = name: vm: ''
    ### nixling-generate-keys: ${name}
    vm_keys_dir="${cfg.site.keysDir}"
    vm_state_dir="${cfg.site.stateDir}/vms/${name}"
    vm_host_keys_dir="$vm_state_dir/host-keys"
    priv="$vm_keys_dir/${name}_ed25519"
    pub="$priv.pub"

    install -d -m 0710 -o root -g nixling "$vm_keys_dir"
    install -d -m 3770 -o nixlingd -g users "$vm_state_dir" 2>/dev/null || true
    install -d -m 0750 -o nixlingd -g nixling "$vm_host_keys_dir"
    chmod g-s "$vm_host_keys_dir"

    if [ ! -f "$priv" ]; then
      umask 077
      staging="$vm_keys_dir/.${name}_ed25519.tmp.$$"
      # ssh-keygen reads its output path; we ensure the staging file
      # does not pre-exist so it can write a fresh keypair.
      ${pkgs.coreutils}/bin/rm -f "$staging" "$staging.pub"
      if ! ${pkgs.openssh}/bin/ssh-keygen \
            -t ed25519 -N "" \
            -C "nixling:${name}" \
            -f "$staging" >/dev/null; then
        echo "nixling-generate-keys: FAILED to generate $priv" >&2
        ${pkgs.coreutils}/bin/rm -f "$staging" "$staging.pub"
        exit 1
      fi
      mv -T -- "$staging.pub" "$pub"
      mv -T -- "$staging"     "$priv"
    fi

    # Repair modes on every activation. Idempotent.
    # Owner = root, group = nixling, mode = 0640.
    # ssh's identity-file permission check requires the caller to
    # OWN the file or the file to be 0600 with no group/other. We
    # can't satisfy either here (root owns, launcher-group reads).
    # The CLI's vmLaunchScript copies the key to a per-launch
    # tempfile in $XDG_RUNTIME_DIR with the caller's UID + 0600
    # before passing it to ssh. The 0640 mode lets the copy work
    # without sudo.
    chown root:nixling "$priv"
    chmod 0640 "$priv"
    chown root:root "$pub"
    chmod 0644 "$pub"

    # Stage the per-VM host-keys share for the guest to mount.
    install -m 0644 -o root -g root "$pub" "$vm_host_keys_dir/host.pub"
    user_keys_tmp=$(${pkgs.coreutils}/bin/mktemp \
      "$vm_host_keys_dir/.user-authorized-keys.XXXXXX")
    cat > "$user_keys_tmp" <<'NIXLING_USER_KEYS_EOF'
${perVmUserAuthorizedKeysText vm}
NIXLING_USER_KEYS_EOF
    chmod 0644 "$user_keys_tmp"
    chown root:root "$user_keys_tmp"
    mv -T -- "$user_keys_tmp" "$vm_host_keys_dir/user-authorized-keys"
  '';

  generateKeysBody = lib.concatStringsSep "\n"
    (lib.mapAttrsToList perVmGenScript normalNixosVms);
in
{
  # The keys directory itself + the lock file. Pre-created with
  # `systemd.tmpfiles` so the activation script can flock the lock
  # without racing with the first-ever activation's directory
  # creation.
  systemd.tmpfiles.rules = [
    # Mode 0710 + group nixling: the owning group's --x bit
    # grants directory traversal so launcher-group members can stat +
    # read private keys inside (each key carries its own
    # group:nixling:r-- ACL). Prior to this fix the mode was
    # 0700 root:root and a named-group ACL (group:nixling:--x)
    # provided traverse. That broke because: (a) `d 0700` forces the
    # POSIX ACL mask to --- which neutralizes named-group entries, and
    # (b) systemd-tmpfiles skips the `a+` fix-up rule due to the
    # microvm→root ownership transition on /var/lib/nixling. Using
    # traditional group bits avoids ACLs entirely.
    "d ${cfg.site.keysDir}             0710 root nixling -"
    "f ${cfg.site.keysDir}/.lock       0600 root root -"
  ];

  # Generate + repair on every activation. Ordering:
  #   - After `users`: needs the nixling group to exist for
  #     the ACL grant on the private key.
  system.activationScripts.nixlingGenerateKeys = lib.stringAfter [ "users" ] ''
    set -u

    # Serialise: two concurrent nixos-rebuild switches must not race
    # on ssh-keygen for the same VM. 60s timeout is generous — keygen
    # is essentially instant; the lock is only ever contended in CI /
    # parallel rebuild scenarios.
    exec 9>"${cfg.site.keysDir}/.lock"
    if ! ${pkgs.util-linux}/bin/flock -w 60 9; then
      echo "nixling-generate-keys: failed to acquire ${cfg.site.keysDir}/.lock within 60s" >&2
      exit 1
    fi

    ${generateKeysBody}

    # Grant the nixling group traverse-only on the keys
    # directory itself so members can stat + read the individual key
    # files (which are chown'd root:nixling 0640). This MUST
    # run AFTER generateKeysBody because each VM's per-script call
    # does `install -d -m 0700` which resets the dir mode and strips
    # ACLs from the effective permission mask.
    ${pkgs.acl}/bin/setfacl -m "g:nixling:--x" "${cfg.site.keysDir}" || true
    ${pkgs.acl}/bin/setfacl -m "m::r-x" "${cfg.site.keysDir}" || true
  '';
}
