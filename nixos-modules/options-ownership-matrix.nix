# typed declaration of the per-VM state
# directory ownership matrix.
#
# Every leaf path under /var/lib/d2b/vms/<vm>/ has a canonical
# owner/group/mode. The matrix is cross-referenced against the
# `writablePaths` blocks in nixos-modules/minijail-profiles.nix; the
# minijail profiles describe WHAT a runner role may write, this
# matrix describes WHO OWNS each subdirectory and WITH WHICH MODE.
#
# CRITICAL — hardlink farm carve-out.
# /var/lib/d2b/vms/<vm>/store-view/live/ (and the legacy
# /var/lib/d2b/vms/<vm>/store/) is a hardlink pool whose inodes are
# SHARED with /nix/store. `setfacl -R` (or `chmod -R`, `chown -R`)
# recursively across that subtree propagates ACLs INTO /nix/store via the
# shared inodes, which breaks the openssh `safe_path` checks on the per-VM
# ssh host keys (and any other uid-sensitive consumer of /nix/store). Every
# entry in the matrix below carries an explicit `recursive` field that
# defaults to `false`; the `store` and `store-view/live` entries MUST keep
# `recursive = false` and the enforcer asserts the carve-out independently.
#
# Signed store-view layout (ADR 0027): `store-view/{live,meta}` are
# runner/virtiofsd-readable (`d2bd:users 0755`); `store-view/state`,
# `store-view/state/generations`, and `store-view/gcroots` are HOST-ONLY
# (`d2bd:d2b 0750`) and MUST NOT reuse the runner-readable
# `users 0755` posture; `store-view/sync.lock` is broker-private
# (`d2bd:d2b 0600`, file-kind); the live readiness marker is
# guest-readable (`d2bd:users 0644`, file-kind).
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
      kind ? "dir",
      required ? true,
      recursive ? false,
      description,
    }:
    {
      inherit
        path
        owner
        group
        mode
        kind
        required
        recursive
        description
        ;
    };

  defaultMatrix = [
    (mkEntry {
      path = ".";
      owner = "d2bd";
      group = "users";
      mode = "3770";
      description = ''
        Per-VM state root. setgid so role users (runner / gpu / swtpm)
        inherit the group on files they create inside the directory.
      '';
    })
    (mkEntry {
      path = "state";
      owner = "d2bd";
      group = "d2b";
      mode = "0750";
      description = "Daemon-owned per-VM state subdirectory (audio-state.json, etc.).";
    })
    (mkEntry {
      path = "swtpm";
      owner = "d2b-<vm>-swtpm";
      group = "d2b-<vm>-swtpm";
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
      owner = "d2bd";
      group = "d2b";
      mode = "0750";
      description = ''
        Container for per-VM sshd host keys. The daemon refuses to start
        the VM if any leaf has drifted.
      '';
    })
    (mkEntry {
      path = "host-keys";
      owner = "d2bd";
      group = "d2b";
      mode = "0750";
      description = "Known-hosts pin store for per-VM ssh host key fingerprints.";
    })
    (mkEntry {
      path = "store";
      owner = "d2bd";
      group = "users";
      mode = "0755";
      recursive = false;
      required = false;
      description = ''
        LEGACY RECOVERY ARTIFACT (ADR 0027): pre-store-view per-VM
        /nix/store hardlink farm. Inodes are SHARED with /nix/store;
        recursive ownership/ACL ops here propagate INTO /nix/store and
        break openssh safe_path() checks on the per-VM ssh host keys.
        The enforcer MUST NEVER recurse into this subdirectory.
        `recursive` is hard-pinned to false and the daemon-side
        enforcer additionally asserts the carve-out by name (see
        packages/d2b-host/src/ownership_matrix.rs). `required` is
        false: native (post-cutover) VMs never had this artifact, so
        its absence must not fail preflight; migrated VMs still have
        it checked/postured when present.
      '';
    })
    (mkEntry {
      path = "store-meta";
      owner = "d2bd";
      group = "users";
      mode = "0755";
      recursive = false;
      required = false;
      description = ''
        LEGACY RECOVERY ARTIFACT (ADR 0027): pre-store-view StoreSync
        metadata sibling to `store/`. Held the `current` symlink,
        per-generation marker, and gcroots. Retained only while
        migration support exists; `required` is false so native VMs
        without it pass preflight. Enforcer keeps `recursive = false`.
      '';
    })
    (mkEntry {
      path = "store-view";
      owner = "d2bd";
      group = "users";
      mode = "0755";
      recursive = false;
      description = ''
        Canonical per-VM store-view root (ADR 0027). Holds the served
        `live/` hardlink pool, the guest-readable `meta/` subtree, and
        the host-only `state/`, `gcroots/`, and `sync.lock`. Must not
        inherit broad writable default ACLs from the per-VM state root.
      '';
    })
    (mkEntry {
      path = "store-view/live";
      owner = "d2bd";
      group = "users";
      mode = "0755";
      recursive = false;
      description = ''
        CRITICAL CARVE-OUT: canonical per-VM /nix/store hardlink live pool.
        Inodes under package trees are SHARED with /nix/store; recursive
        ownership/ACL ops here propagate into /nix/store. The enforcer MUST
        NEVER recurse into this subdirectory. Served read-only to the guest
        as /nix/.ro-store; the runner/virtiofsd identity needs read access
        (`users 0755`).
      '';
    })
    (mkEntry {
      path = "store-view/meta";
      owner = "d2bd";
      group = "users";
      mode = "0755";
      recursive = false;
      description = ''
        Guest read-only metadata share root (ADR 0027). Served read-only
        as /run/d2b-store-meta. Holds only the guest-safe `current`
        symlink and `generations/<id>/{store-paths,db.dump,meta.json}`.
        Runner/virtiofsd-readable (`users 0755`); never exposes `live/`,
        `state/`, `gcroots/`, or the broker `sync.lock`.
      '';
    })
    (mkEntry {
      path = "store-view/meta/generations";
      owner = "d2bd";
      group = "users";
      mode = "0755";
      recursive = false;
      description = ''
        Guest-readable per-generation metadata directory under
        `store-view/meta`. Runner/virtiofsd-readable (`users 0755`).
        Per-generation leaves are broker-written and verified out of
        band with directory-only operations.
      '';
    })
    (mkEntry {
      path = "store-view/state";
      owner = "d2bd";
      group = "d2b";
      mode = "0750";
      recursive = false;
      description = ''
        HOST-ONLY broker StoreSync state (ADR 0027). `d2b:d2b
        0750` so the runner/virtiofsd identity has no access — this is
        broker-authoritative metadata (`current`, per-generation
        marker.json/meta.json/integrity.json, integrity-unknown.json)
        that must never reach the guest. Must NOT reuse the
        runner-readable `users 0755` store-view posture.
      '';
    })
    (mkEntry {
      path = "store-view/state/generations";
      owner = "d2bd";
      group = "d2b";
      mode = "0750";
      recursive = false;
      description = ''
        HOST-ONLY per-generation broker state directory under
        `store-view/state`. `d2b:d2b 0750`. Per-generation
        host-only leaves (`marker.json`, `meta.json`, `integrity.json`)
        are `d2b:d2b 0640`, repaired out of band; they are not
        enumerated here because `<id>` is dynamic.
      '';
    })
    (mkEntry {
      path = "store-view/gcroots";
      owner = "d2bd";
      group = "d2b";
      mode = "0750";
      recursive = false;
      description = ''
        HOST-ONLY StoreSync GC roots (ADR 0027). `d2b:d2b 0750`.
        Holds host-absolute symlinks into /nix/store that protect
        retained closures from host GC; never guest- or runner-readable.
      '';
    })
    (mkEntry {
      path = "store-view/sync.lock";
      owner = "d2bd";
      group = "d2b";
      mode = "0600";
      kind = "file";
      description = ''
        BROKER-PRIVATE StoreSync serialization lock (ADR 0027).
        `d2b:d2b 0600`, file-kind: the enforcer reasserts
        mode/uid/gid on the file inode with no-follow semantics and
        never recurses. Created by broker prep before ownership
        preflight on a fresh VM.
      '';
    })
    (mkEntry {
      path = "store-view/state/integrity-unknown.json";
      owner = "d2bd";
      group = "d2b";
      mode = "0640";
      kind = "file";
      required = false;
      description = ''
        HOST-ONLY VM-level integrity fallback record (ADR 0027), used
        when generation identity is indeterminate. `d2b:d2b
        0640`, file-kind. `required` is false: it is created lazily by
        broker integrity code, so its absence before first use must not
        fail preflight. Other (non-ENOENT) stat errors still drift.
      '';
    })
    (mkEntry {
      path = "store-view/live/.d2b-marker-<vm>";
      owner = "d2bd";
      group = "users";
      mode = "0644";
      kind = "file";
      required = false;
      description = ''
        Guest-readable live readiness marker (ADR 0027). Zero-length;
        `d2b:users 0644` so the guest/runner may read it through the
        read-only `live/` share but only the broker may write it.
        File-kind single-inode check: explicitly exempt from the
        no-recursion-into-`live/` carve-out (a direct stat of one named
        file is not a recursive walk and never touches package trees).
        `required` is false: it is absent before the first successful
        StoreSync.
      '';
    })
  ];
in
{
  options.d2b.daemon.perVmStateOwnershipMatrix = mkOption {
    description = ''
      Typed ownership matrix for every per-VM state subdirectory under
      `/var/lib/d2b/vms/<vm>/`. Consumed by the daemon's VM-start
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
            Subdirectory path relative to `/var/lib/d2b/vms/<vm>/`.
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
        kind = mkOption {
          type = types.enum [ "dir" "file" ];
          default = "dir";
          description = ''
            Whether the entry is a directory (`dir`, default) or a
            regular file (`file`). The daemon enforcer stats the path
            with no-follow `symlink_metadata`: a `file` entry MUST be a
            regular file when present and is never walked recursively;
            a `dir` entry MUST be a directory. File-kind entries still
            reassert owner/group/mode on the file inode.
          '';
        };
        required = mkOption {
          type = types.bool;
          default = true;
          description = ''
            If true (default), the entry is expected to exist by the
            time the daemon runs its VM-start preflight (broker prep
            creates required paths first). If false, the entry is
            posture-if-present: only a not-found (`ENOENT`) stat result
            is skipped; every other stat error (`EACCES`, `EIO`,
            `ELOOP`, …) still surfaces as drift/error. Use `false` for
            lazily-created paths (`store-view/live/.d2b-marker-<vm>`,
            `store-view/state/integrity-unknown.json`) and for legacy
            recovery artifacts (`store`, `store-meta`) absent on
            native post-cutover VMs.
          '';
        };
        recursive = mkOption {
          type = types.bool;
          default = false;
          description = ''
            If true, the enforcer recurses into the subdirectory when
            checking ownership/mode. MUST default to false. MUST stay
            false for `store` and `store-view/live` (per-VM /nix/store
            hardlink pools whose inodes are shared with /nix/store — see
            the module-level critical-carve-out comment). Ignored for
            `file`-kind entries (a single inode is never walked).
          '';
        };
        description = mkOption {
          type = types.str;
          description = "Operator-facing rationale for this entry.";
        };
      };
    });
    default = defaultMatrix;
    defaultText = lib.literalExpression "the canonical /var/lib/d2b/vms/<vm>/ ownership matrix";
  };
}
