# Host-managed sshd host keys for d2b VMs.
#
# Why
# ---
# Without this module each VM regenerates its sshd host keys on first
# boot and stores them on the tmpfs overlay over the read-only nix
# store, which means:
#   - the keys are EPHEMERAL: every VM restart regenerates them.
#   - the host's `d2b-known-hosts-refresh.service` pins the
#     first observed key in `known_hosts.d2b` and refuses to
#     overwrite on subsequent restarts (correctly: from the host's
#     point of view a host-key change IS a possible MITM/swap).
#   - the result is that any host-keys regeneration soft-bricks the
#     framework's automated SSH path until an operator runs
#     `ssh-keygen -R <ip>` and restarts the refresh service.
#
# What
# ----
# This module mirrors the `host-keys.nix` per-VM client-key pattern,
# but for the *server* side. For every enabled `d2b.vms.<name>`:
#
#   - Generate at host activation:
#       ${site.stateDir}/vms/<name>/sshd-host-keys/ssh_host_ed25519_key      (0400 root)
#       ${site.stateDir}/vms/<name>/sshd-host-keys/ssh_host_ed25519_key.pub  (0644 root)
#     Atomic install via tempfile + mv. Idempotent.
#
#   - `store.nix` shares the dir read-only into the guest as
#     `/run/d2b-sshd-host-keys/` (virtiofs tag `d2b-ssh-host`).
#
#   - `host.nix` injects a small NixOS-module fragment into every
#     enabled VM that points `services.openssh.hostKeys` at the
#     in-guest mount path and disables sshd's auto-generation
#     (`services.openssh.generateHostKeys = false`). That fragment
#     lives in `./guest-sshd-host-keys.nix`.
#
#   - `host-known-hosts.nix` reads the host-side pubkey directly
#     instead of probing the live VM. The pinned known_hosts entry
#     is authoritative from generation 1 — no TOFU drift on restart.
{ config, pkgs, lib, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  normalNixosVms = d2bLib.normalNixosVms cfg.vms;

  # Path strings used both here AND in store.nix's share declaration
  # AND in guest-sshd-host-keys.nix. Encoded as a `let` here so the
  # other call sites can re-derive identical paths from the same VM
  # name without re-stating the convention. They are not exported
  # as options — this is a framework-internal contract.
  perVmHostKeysDir = name: "${cfg.site.stateDir}/vms/${name}/sshd-host-keys";
  perVmHostKeyPriv = name: "${perVmHostKeysDir name}/ssh_host_ed25519_key";
  perVmHostKeyPub  = name: "${perVmHostKeyPriv name}.pub";

  # Per-VM generation block. Same idempotent atomic-install pattern
  # as host-keys.nix's perVmGenScript: ssh-keygen into a staging path,
  # mv -T into place, repair perms on every activation.
  perVmGenScript = name: _vm: ''
    ### d2b-generate-sshd-host-keys: ${name}
    vm_host_keys_dir=${lib.escapeShellArg (perVmHostKeysDir name)}
    priv=${lib.escapeShellArg (perVmHostKeyPriv name)}
    pub=${lib.escapeShellArg (perVmHostKeyPub name)}

    install -d -m 3770 -o d2bd -g users "${cfg.site.stateDir}/vms/${name}" 2>/dev/null || true
    install -d -m 0750 -o d2bd -g d2b "$vm_host_keys_dir"
    chmod g-s "$vm_host_keys_dir"

    if [ ! -f "$priv" ]; then
      umask 077
      staging="$vm_host_keys_dir/.ssh_host_ed25519_key.tmp.$$"
      ${pkgs.coreutils}/bin/rm -f "$staging" "$staging.pub"
      if ! ${pkgs.openssh}/bin/ssh-keygen \
            -t ed25519 -N "" \
            -C "d2b:${name}:sshd-host-key" \
            -f "$staging" >/dev/null; then
        echo "d2b-generate-sshd-host-keys: FAILED to generate $priv" >&2
        ${pkgs.coreutils}/bin/rm -f "$staging" "$staging.pub"
        exit 1
      fi
      mv -T -- "$staging.pub" "$pub"
      mv -T -- "$staging"     "$priv"
    fi

    # Repair modes on every activation. The priv key must be
    # readable by the in-VM sshd user; sshd inside the guest
    # opens /run/d2b-sshd-host-keys/ssh_host_ed25519_key as
    # root before chrooting, and virtiofs passes through the host
    # owner/mode unchanged. 0400 root:root is correct.
    chown root:root "$priv" "$pub"
    chmod 0400 "$priv"
    chmod 0644 "$pub"
  '';

  generateKeysBody = lib.concatStringsSep "\n"
    (lib.mapAttrsToList perVmGenScript normalNixosVms);
in
{
  # Pre-create the global per-VM state root so the activation script
  # never races on directory creation. (Per-VM dirs are created by
  # `install -d` inside perVmGenScript.)
  systemd.tmpfiles.rules = [
    "d ${cfg.site.stateDir}/vms 0755 root root -"
  ];

  # Generate + repair on every activation. After `users` to keep
  # ordering consistent with `host-keys.nix`; we don't actually need
  # any d2b-* group for this module, but using the same anchor
  # avoids two parallel activation scripts both creating
  # `${site.stateDir}/vms/<name>/`.
  system.activationScripts.d2bGenerateSshdHostKeys = lib.stringAfter [ "users" "d2bGenerateKeys" ] ''
    set -u

    ${generateKeysBody}
  '';
}
