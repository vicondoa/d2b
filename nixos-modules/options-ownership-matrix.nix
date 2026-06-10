# typed declaration of the per-VM state
# directory ownership matrix.
#
# Every leaf path under /var/lib/nixling/vms/<vm>/ has a canonical
# owner/group/mode. The matrix is cross-referenced against the
# `writablePaths` blocks in nixos-modules/minijail-profiles.nix; the
# minijail profiles describe WHAT a runner role may write, this
# matrix describes WHO OWNS each subdirectory and WITH WHICH MODE.
#
# CRITICAL — hardlink farm carve-out.
# /var/lib/nixling/vms/<vm>/store/ is a per-generation hardlink farm
# whose inodes are SHARED with /nix/store. `setfacl -R` (or `chmod -R`,
# `chown -R`) recursively across that subtree propagates ACLs INTO
# /nix/store via the shared inodes, which breaks the openssh
# `safe_path` checks on the per-VM ssh host keys (and any other
# uid-sensitive consumer of /nix/store). Every entry in the matrix
# below carries an explicit `recursive` field that defaults to `false`;
# the `store` entry MUST keep `recursive = false` and the enforcer
# asserts the carve-out independently.
{ lib, ... }:

let
  inherit (lib) mkOption types;

  # `<vm>` in `owner`/`group` is substituted by the daemon enforcer at
  # check time. The Nix layer stays VM-agnostic so the matrix is a
  # single static value shared by every VM.
  mkEntry =
    {
      path,
      owner,
      group,
      mode,
      recursive ? false,
      description,
    }:
    {
      inherit path owner group mode recursive description;
    };

  defaultMatrix = [
    (mkEntry {
      path = ".";
      owner = "nixlingd";
      group = "users";
      mode = "2770";
      description = ''
        Per-VM state root. setgid so role users (runner / gpu / swtpm)
        inherit the group on files they create inside the directory.
      '';
    })
    (mkEntry {
      path = "state";
      owner = "nixlingd";
      group = "nixling";
      mode = "0750";
      description = "Daemon-owned per-VM state subdirectory (audio-state.json, etc.).";
    })
    (mkEntry {
      path = "swtpm";
      owner = "nixling-<vm>-swtpm";
      group = "nixling-<vm>-swtpm";
      mode = "0700";
      description = ''
        CRITICAL SUBSYSTEM (AGENTS.md): per-VM TPM 2.0 NVRAM. Wiping or
        rechowning this directory looks like device tampering to any
        IdP (Entra ID / Intune / BitLocker-class policies) and forces
        re-enrollment. Owned by the per-VM swtpm runner principal.
      '';
    })
    (mkEntry {
      path = "sshd-host-keys";
      owner = "nixlingd";
      group = "nixling";
      mode = "0750";
      description = ''
        Container for per-VM sshd host keys. The daemon refuses to start
        the VM if any leaf has drifted.
      '';
    })
    (mkEntry {
      path = "host-keys";
      owner = "nixlingd";
      group = "nixling";
      mode = "0750";
      description = "Known-hosts pin store for per-VM ssh host key fingerprints.";
    })
    (mkEntry {
      path = "store";
      owner = "nixlingd";
      group = "users";
      mode = "0755";
      recursive = false;
      description = ''
        CRITICAL CARVE-OUT: per-VM /nix/store hardlink farm. Inodes are
        SHARED with /nix/store; recursive ownership/ACL ops here
        propagate INTO /nix/store and break openssh safe_path() checks
        on the per-VM ssh host keys. The enforcer MUST NEVER recurse
        into this subdirectory. `recursive` is hard-pinned to false and
        the daemon-side enforcer additionally asserts the carve-out by
        name (see packages/nixling-host/src/ownership_matrix.rs).
      '';
    })
    (mkEntry {
      path = "store-view";
      owner = "nixlingd";
      group = "users";
      mode = "0755";
      recursive = false;
      description = ''
        Canonical per-VM store-view root. Contains metadata-only
        `generations/` plus the served `live/` hardlink pool. Must not
        inherit broad writable default ACLs from the per-VM state root.
      '';
    })
    (mkEntry {
      path = "store-view/live";
      owner = "nixlingd";
      group = "users";
      mode = "0755";
      recursive = false;
      description = ''
        CRITICAL CARVE-OUT: canonical per-VM /nix/store hardlink live pool.
        Inodes under package trees are SHARED with /nix/store; recursive
        ownership/ACL ops here propagate into /nix/store. The enforcer MUST
        NEVER recurse into this subdirectory.
      '';
    })
    (mkEntry {
      path = "store-view/generations";
      owner = "nixlingd";
      group = "users";
      mode = "0755";
      recursive = false;
      description = ''
        Metadata-only generation directory for store-view. Per-generation
        leaves are repaired out-of-band with directory-only operations.
      '';
    })
    (mkEntry {
      path = "store-meta";
      owner = "nixlingd";
      group = "users";
      mode = "0755";
      recursive = false;
      description = ''
        StoreSync metadata sibling to `store/`. Holds the `current`
        symlink, per-generation marker, and gcroots. Although the
        contents are not hardlinks into /nix/store, the enforcer keeps
        `recursive = false` and verifies leaves out-of-band so the
        rule "no recursive ownership ops on per-VM store state" is
        applied uniformly.
      '';
    })
  ];
in
{
  options.nixling.daemon.perVmStateOwnershipMatrix = mkOption {
    description = ''
      Typed ownership matrix for every per-VM state subdirectory under
      `/var/lib/nixling/vms/<vm>/`. Consumed by the daemon's VM-start
      preflight: the daemon refuses to start a VM whose per-VM state
      has drifted from this declaration.

      Override only with extreme caution — every entry interacts with
      the broker-side dispatch and the minijail `writablePaths`
      declarations in nixos-modules/minijail-profiles.nix.

      See docs/reference/per-vm-state-ownership.md for the rationale
      and the per-subdirectory hardlink-farm carve-out documentation.
    '';
    type = types.listOf (types.submodule {
      options = {
        path = mkOption {
          type = types.str;
          description = ''
            Subdirectory path relative to `/var/lib/nixling/vms/<vm>/`.
            Use "." for the per-VM root itself.
          '';
        };
        owner = mkOption {
          type = types.str;
          description = ''
            Expected uid resolved by name. The literal token `<vm>` is
            substituted with the VM's name at enforcement time so the
            matrix stays VM-agnostic.
          '';
        };
        group = mkOption {
          type = types.str;
          description = ''
            Expected gid resolved by name. The literal token `<vm>` is
            substituted with the VM's name at enforcement time.
          '';
        };
        mode = mkOption {
          type = types.strMatching "[0-7]{3,4}";
          description = ''
            Expected mode in octal (3 or 4 digits, e.g. "0750" or
            "2770"). Includes setuid/setgid/sticky bits when relevant.
          '';
        };
        recursive = mkOption {
          type = types.bool;
          default = false;
          description = ''
            If true, the enforcer recurses into the subdirectory when
            checking ownership/mode. MUST default to false. MUST stay
            false for `store` (per-VM /nix/store hardlink farm whose
            inodes are shared with /nix/store — see the module-level
            critical-carve-out comment).
          '';
        };
        description = mkOption {
          type = types.str;
          description = "Operator-facing rationale for this entry.";
        };
      };
    });
    default = defaultMatrix;
    defaultText = lib.literalExpression "the canonical /var/lib/nixling/vms/<vm>/ ownership matrix";
  };
}
