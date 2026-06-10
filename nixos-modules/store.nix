# Per-VM nix store for nixling microVMs.
#
# Background
# ----------
# By default, microvm.nix shares the host's entire /nix/store into
# every guest read-only. That leaks the union of every package and
# every other VM's closure into each VM (and prevents host GC from
# trimming anything a VM references).
#
# This module replaces that share with a per-VM hardlink farm under
# /var/lib/nixling/vms/<vm>/store/ containing ONLY the paths in that VM's
# `system.build.toplevel` closure. Bytes are shared with the host's
# real /nix/store via hard links — zero extra disk usage beyond
# directory entries.
#
# Layout (per VM, under /var/lib/nixling/vms/<vm>/)
#
#   store/                          virtiofs → guest /nix/.ro-store
#     <hash>-bash-5.3p9/            hardlink farm into /nix/store
#     <hash>-nixos-system-…/
#     …
#   store-meta/                     virtiofs → guest /run/nixling-store-meta
#     current → generations/N       symlink (atomic flip on switch)
#     generations/
#       N/                          per-generation pin
#         system    → /nix/store/<hash>-nixos-system-<vm>-…
#         store-paths               newline list of closure paths
#         db.dump                   `nix-store --dump-db` of the closure
#         meta.json                 { generation, timestamp, runner }
#     gcroots/
#       generation-N → /nix/store/<hash>-nixos-system-<vm>-…
#                                   GC root: pins host closure to disk
#
# Cross-mount hardlink trick
# --------------------------
# /nix/store on NixOS is bind-mounted ro,nosuid,nodev on top of itself.
# Hardlinks from /nix/store/<x> to /var/lib/nixling/<y> fail with EXDEV
# even though both are on the same ext4 device, because the source is
# in a different vfsmount (Linux `do_linkat` rejects cross-mount
# linking unconditionally).
#
# The sync helper sidesteps this by running in a private mount
# namespace where /nix/store is lazily unmounted. Inside that
# namespace /nix/store is just a directory on the root mount, so
# hardlinks succeed.
{ config, lib, pkgs, ... }:

let
  cfg = config.nixling;
  # nixling-owned access helpers (see lib.nix).
  nl = import ./lib.nix { inherit lib pkgs; };
  enabledVms = lib.filterAttrs (_: vm: vm.enable) cfg.vms;

  # spectrum-ch package (cloud-hypervisor v52 with virtio-gpu patches).
  # Provides ch-remote v52 used by the vfsd-watchdog direct-launch path.
  spectrumCh = import ../pkgs/spectrum-ch { inherit pkgs; };

  # The Wayland user's user-manager handle for `systemctl --user
  # --machine <wuser>@.host`. The watchdog (further down) uses this
  # to reach scopes spawned by interactive `nixling up`. When
  # `waylandUser` is unset (headless deployment), the watchdog
  # bails out before invoking systemctl --user, so the "root@.host"
  # fallback is never actually used; it just keeps the script body
  # syntactically well-formed at eval time.
  userMachine =
    if cfg.site.waylandUser != null
    then "${cfg.site.waylandUser}@.host"
    else "root@.host";


  # Pull the fully-evaluated guest config for each VM out of microvm.nix.
  # `microvm.vms.<name>.config.config` is the evaluated guest NixOS
  # config (the inner `.config` is the standard "you want the config
  # attrset, not the module function"). `system.build.toplevel` is the
  # nixos-system-<vm> derivation that virtiofsd would otherwise be
  # serving from the host's /nix/store. We snapshot per-VM closure
  # info via `pkgs.closureInfo`, which gives us
  #   <out>/store-paths   newline list of every path in the closure
  #   <out>/registration  format consumed by `nix-store --load-db`
  vmTopOf = name: nl.vmToplevel config name;

  # The microvm.nix-generated "runner" derivation for this VM. It
  # holds
  #   bin/virtiofsd-run       (what microvm-virtiofsd@<vm>.service execs)
  #   bin/microvm-run         (what microvm@<vm>.service execs)
  #   bin/microvm-shutdown
  #   bin/tap-up / tap-down
  # plus their transitive closure (bash, supervisord, virtiofsd, ...).
  # These run on the HOST, not the guest, BUT they must be reachable
  # via /nix/store inside the daemon's mount namespace once we
  # bind-mount our per-VM store on top of /nix/store — otherwise
  # systemd's ExecStart= can't resolve the runner script via the
  # symlink at /var/lib/nixling/vms/<vm>/current/bin/virtiofsd-run.
  # Including the runner closure as additional hardlinks costs zero
  # disk (still hardlinks into the host /nix/store) and exposes only
  # the runner binaries to the guest, which are public nixpkgs
  # packages with no host-private data.
  vmRunnerOf = name: nl.vmDeclaredRunner config name;

  # Wrapper around microvm.nix's bin/virtiofsd-run that sanitises the
  # supervisord config before invoking it. The microvm.nix-generated
  # supervisord conf has a top-level `user=root` directive (because
  # the unit historically ran as root). With the C1a User= drop to
  # `nl-virtiofs-<vm>` the kernel rejects supervisord's attempt to
  # setuid to root and supervisord aborts with
  #
  #     Error: Can't drop privilege as nonroot user
  #
  # Stripping the `user=root` line lets supervisord run as the
  # systemd-specified user; the [program:] sections inherit, so the
  # child virtiofsd processes also run as `nl-virtiofs-<vm>`. The
  # sanitised conf is written to /var/lib/nixling/vms/<vm>/ which is
  # writable via ReadWritePaths in the same drop-in.
  #
  # The wrapper script + its closure (bash, sed, supervisord) get
  # pulled into vmClosureOf below so they are visible inside the
  # unit's bind-mounted per-VM /nix/store.
  #
  # What the wrapper actually does
  #
  #   1. Locate microvm.nix's generated virtiofsd-run script + the
  #      supervisord conf it references.
  #   2. For each `command=<path>` entry in that conf (one per
  #      virtiofs share), copy <path> to a writable per-VM location
  #      and sed-patch its hard-coded virtiofsd flags
  #
  #        --inode-file-handles=prefer  ->  --inode-file-handles=never
  #          (C1b — eliminates the daemon's runtime need for
  #          CAP_DAC_READ_SEARCH, which means we can drop that cap
  #          from the bounding set entirely below.)
  #
  #        --posix-acl --xattr          ->  (removed)
  #          (Shrinks the FUSE wire-protocol surface. /nix/store
  #          paths don't use POSIX ACLs or user xattrs.)
  #
  #   3. Sanitise the conf: strip the `user=root` directive that
  #      microvm.nix's generator emits. Harmless when running as
  #      root, but lets a future User= drop work without an
  #      additional change.
  #   4. exec supervisord against the rewritten conf.
  #
  # The patched daemon scripts live under
  # /var/lib/nixling/vms/<vm>/virtiofsd-<base>-hardened so the host's
  # /nix/store remains untouched.
  nixlingVfsRunnerOf = name: pkgs.writeShellApplication {
    name = "nixling-virtiofsd-run-${name}";
    runtimeInputs = with pkgs; [
      coreutils
      gnused
      gnugrep
      python3Packages.supervisor
    ];
    text = ''
      set -eu

      # Belt-and-suspenders marker gate. Two independent layers must both
      # succeed before any virtiofsd child is spawned
      #
      #   1. ExecStartPre on the systemd unit (see store.nix) runs with
      #      `+` (host namespace, root, before any sandboxing) and
      #      tests the SOURCE marker at
      #        /var/lib/nixling/vms/<vm>/store/.nixling-marker-<vm>
      #      -- proving nixling-store-sync actually planted the file.
      #
      #   2. The check below runs as ExecStart (inside the unit's
      #      mount namespace, AFTER BindReadOnlyPaths has remapped
      #      /nix/store -> /var/lib/nixling/vms/<vm>/store) and tests the
      #      BIND-VIEW marker at
      #        /nix/store/.nixling-marker-<vm>
      #      -- proving BindReadOnlyPaths actually took effect.
      #
      # Both layers are necessary because each catches a different
      # failure mode. A source-only check still passes if a future
      # systemd refactor (or microvm.nix upstream change) silently
      # no-ops BindReadOnlyPaths -- the marker remains on the host's
      # /var/lib, but /nix/store inside the namespace is the host's
      # REAL store, and virtiofsd would happily serve the host's
      # entire /nix/store to the guest. A namespace-only check still
      # passes for any contents anyone planted into the per-VM store
      # directory without going through nixling-store-sync. Together
      # the two layers establish: the marker was planted by the sync
      # helper AND the daemon is serving that exact view.
      if [ ! -e "/nix/store/.nixling-marker-${name}" ]; then
        echo "nixling-virtiofsd-run-${name}: bind-view marker /nix/store/.nixling-marker-${name} missing; BindReadOnlyPaths did not take effect or the per-VM store is empty -- refusing to start virtiofsd" >&2
        exit 3
      fi

      ORIG=${cfg.store.stateDir}/${name}/current/bin/virtiofsd-run
      if [ ! -x "$ORIG" ]; then
        echo "nixling-virtiofsd-run-${name}: missing $ORIG" >&2
        exit 2
      fi
      CONF=$(sed -nE 's|.*--configuration[[:space:]]+([^[:space:]]+).*|\1|p' "$ORIG" | head -n1)
      if [ -z "$CONF" ] || [ ! -f "$CONF" ]; then
        echo "nixling-virtiofsd-run-${name}: could not locate supervisord conf in $ORIG" >&2
        exit 2
      fi

      # Hardened scripts live in /run (root-owned tmpfs, 0700) instead
      # of /var/lib/nixling/vms/<vm>/ which is mode 2775 group=kvm for
      # graphics VMs. A kvm-group process could otherwise rename/swap
      # the patched script between write and supervisord-exec.
      # Security hardening.
      #
      # NEVER create or chmod
      # /run/nixling here — host-daemon.nix owns that path when the
      # daemon is enabled, and host.nix's tmpfiles owns it in the
      # pre-daemon path. We only touch the per-VM leaf under it.
      HARDENED=/run/nixling/${name}/hardened
      install -d -m 0700 -o root -g root /run/nixling/${name}
      install -d -m 0700 -o root -g root "$HARDENED"
      LOCAL=$HARDENED/supervisord-hardened.conf

      # Start from a writable copy of the conf; strip user=root.
      sed '/^user=root$/d' "$CONF" > "$LOCAL.tmp"

      # Per command= entry, copy + patch the referenced script.
      # The script is a small shell wrapper microvm.nix generates
      # that exec's the actual virtiofsd binary with the share's
      # flags. We sed those flags in-place AND add --modcaps to
      # drop virtiofsd's runtime retain-set + --readonly to refuse
      # any guest write attempt.
      #
      # The retain set we drop
      #   chown, dac_override, fowner, fsetid, setuid, setgid,
      #   mknod, setfcap.
      # cap_dac_read_search is already dropped via
      # CapabilityBoundingSet at the systemd level; this is the
      # in-process complement so the running daemon process AND its
      # serving-children all have CapEff=0 post-init.
      # --readonly: virtiofsd refuses every write at the FUSE wire
      # level. The nixling guests treat /nix/.ro-store and
      # /run/nixling-store-meta as read-only mounts, so this is
      # never reached by legitimate guest traffic.
      while IFS= read -r origcmd; do
        [ -n "$origcmd" ] || continue
        if [ ! -f "$origcmd" ]; then
          echo "nixling-virtiofsd-run-${name}: command path missing: $origcmd" >&2
          exit 2
        fi
        base=$(basename "$origcmd")
        patched=$HARDENED/$base-hardened

        # A microvm.nix supervisord conf can also list non-virtiofsd
        # commands (e.g. a Python event handler). Only the virtiofsd
        # invocations have to carry our hardening flags, so gate the
        # fail-closed check below on a STRUCTURAL signal of "this is
        # a virtiofsd invocation": either the script's `exec` line
        # invokes a binary whose path contains `virtiofsd`, or it
        # carries the `--shared-dir` flag that virtiofsd uniquely
        # requires.
        #
        # The previous gate keyed off the literal `--inode-file-handles`
        # token. That worked because microvm.nix's generator always
        # emits it, but if upstream ever drops the flag, picks a
        # different default, or splits its generator we silently
        # skip the fail-closed check and exec an unhardened daemon.
        # The two signals below are durable: `exec virtiofsd` is the
        # whole point of these scripts, and `--shared-dir` has no
        # alternative spelling. Non-virtiofsd commands (the Python
        # supervisord-event-handler) match neither, stay at
        # needs_hardening=0, and pass through unmodified.
        needs_hardening=0
        if grep -qm1 -E '^exec[[:space:]]+.*virtiofsd' "$origcmd" \
           || grep -qF -- '--shared-dir' "$origcmd"; then
          needs_hardening=1
        fi

        sed -e 's/--inode-file-handles=prefer/--inode-file-handles=never --modcaps=-chown:-dac_override:-fowner:-fsetid:-setuid:-setgid:-mknod:-setfcap --readonly/g' \
            -e 's/ --posix-acl//g' \
            -e 's/--posix-acl //g' \
            -e 's/--posix-acl$//g' \
            -e 's/ --xattr//g' \
            -e 's/--xattr //g' \
            -e 's/--xattr$//g' \
            "$origcmd" > "$patched.tmp"

        # Fail-closed verification.
        #
        # The sed substitution that injects --modcaps and --readonly
        # is anchored on the literal string `--inode-file-handles=prefer`.
        # If microvm.nix ever changes its generator to emit `=never`
        # instead of `=prefer`, or to split the flag across two
        # arguments (`--inode-file-handles never`), the substitution
        # silently no-ops -- the patched script reaches supervisord
        # without --readonly and without --modcaps, and virtiofsd
        # starts with its default retain-set and write-capable wire
        # protocol. The hardening would regress with no log.
        #
        # So after patching, independently of HOW the flags ended up
        # in the patched file (our sed, upstream pre-application, or
        # any future mechanism), require all three hardening tokens
        # to be present before we let the script run. Anything else
        # exits 4, the unit's ExecStart fails, and supervisord is
        # never reached with an unhardened virtiofsd command.
        #
        # This is also forward-compatible: if microvm.nix ever
        # applies one of these flags upstream, the grep still passes
        # (it verifies presence, not authorship).
        if [ "$needs_hardening" = 1 ]; then
          for flag in \
            '--inode-file-handles=never' \
            '--readonly' \
            '--modcaps=-chown:-dac_override:-fowner:-fsetid:-setuid:-setgid:-mknod:-setfcap'
          do
            if ! grep -qF -- "$flag" "$patched.tmp"; then
              echo "nixling-virtiofsd-run-${name}: hardening flag $flag missing from patched script $base; refusing to exec" >&2
              rm -f "$patched.tmp"
              exit 4
            fi
          done
        fi

        chmod 0500 "$patched.tmp"
        mv -f "$patched.tmp" "$patched"

        # Rewrite the conf entry to reference the patched copy.
        # Use a delimiter unlikely to appear in /nix/store paths.
        orig_esc=$(printf '%s' "$origcmd" | sed -e 's/[\\|]/\\&/g')
        patched_esc=$(printf '%s' "$patched" | sed -e 's/[\\|]/\\&/g')
        sed -i "s|^command=$orig_esc\$|command=$patched_esc|" "$LOCAL.tmp"
      done < <(grep -oE '^command=[^[:space:]]+' "$CONF" | sed -e 's/^command=//')

      chmod 0500 "$LOCAL.tmp"
      mv -f "$LOCAL.tmp" "$LOCAL"
      exec supervisord --configuration "$LOCAL"
    '';
  };

  vmClosureOf = name:
    let
      runner = vmRunnerOf name;
    in
    pkgs.closureInfo {
      rootPaths = [ (vmTopOf name) (nixlingVfsRunnerOf name) ]
        ++ lib.optional (runner != null) runner;
    };

  # Eval-time generation directory for a VM. The host activation
  # script materialises this under
  #   /var/lib/nixling/vms/<vm>/store-meta/generations/<N>/
  # by copying everything inside the derivation. Keeping it as a
  # plain derivation makes it a first-class Nix value (so it's
  # automatically GC-rooted through the host's system closure) and
  # makes test assertions trivial.
  vmGenerationOf = name:
    let
      top = vmTopOf name;
      closure = vmClosureOf name;
    in
    pkgs.runCommand "nixling-${name}-generation" { } ''
      mkdir -p $out
      ln -s ${top} $out/system
      cp ${closure}/store-paths $out/store-paths
      cp ${closure}/registration $out/db.dump
    '';

  # ---------------------------------------------------------------------------
  # The host-side sync helper. One script, invoked from systemd and
  # from `nixling build/switch/...`. Idempotent — re-running is cheap.
  #
  # Usage
  #   nixling-store-sync <vm> <generation-dir>
  #     <vm>              VM name
  #     <generation-dir>  derivation output from vmGenerationOf
  #
  # Steps
  #   1. Enter a private mount namespace; lazily unmount /nix/store so
  #      hardlinks from /nix/store/<x> to /var/lib/nixling/<x> work
  #      (see "Cross-mount hardlink trick" above).
  #   2. Compute the next generation number = max(existing) + 1.
  #   3. Stage everything under store-stage-<N>/ first; only swap in
  #      after success.
  #   4. For every path in the new closure: hardlink into
  #      store-stage-<N>/<basename>/ if not already in store/<basename>/.
  #      Use `cp -al` (recursive hardlink).
  #   5. Move store-stage-<N>/* into store/ (rename within same dir =
  #      atomic per-path).
  #   6. Write store-meta/generations/<N>/{system,store-paths,db.dump,meta.json}.
  #   7. Place a GC root under store-meta/gcroots/generation-<N>.
  #   8. Atomic-rename store-meta/current → generations/<N>.
  #   9. Retention: compute the kept set
  #        - <N>                           the new generation
  #        - <R> matching the running VM, if any  (parse cmdline)
  #          (or the most-recent prior generation if VM down)
  #      Remove every other store-meta/generations/<K>/ AND every
  #      gcroots/generation-<K> link.
  #  10. Sweep store/: any <hash>-foo not in
  #      union(store-paths of kept generations) → unlink (drops the
  #      hardlink; bytes still live in host /nix/store).
  # ---------------------------------------------------------------------------
  nixlingStoreSync = pkgs.writeShellApplication {
    name = "nixling-store-sync";
    runtimeInputs = with pkgs; [
      coreutils
      util-linux
      gnugrep
      gnused
      gawk
      procps
      nix
      jq
    ];
    excludeShellChecks = [ "SC2016" ];
    text = ''
      set -euo pipefail

      if [ "$(id -u)" -ne 0 ]; then
        echo "nixling-store-sync: must run as root (need mount-namespace + chown)." >&2
        exit 1
      fi

      VM="''${1:-}"
      GEN_SRC="''${2:-}"
      if [ -z "$VM" ] || [ -z "$GEN_SRC" ]; then
        echo "usage: nixling-store-sync <vm> <generation-derivation-dir>" >&2
        exit 2
      fi
      if [ ! -d "$GEN_SRC" ] || [ ! -f "$GEN_SRC/store-paths" ]; then
        echo "nixling-store-sync: '$GEN_SRC' is not a valid generation dir." >&2
        exit 2
      fi

      STATE_DIR=${cfg.store.stateDir}/$VM
      STORE_DIR=$STATE_DIR/store
      META_DIR=$STATE_DIR/store-meta
      GEN_DIR=$META_DIR/generations
      ROOT_DIR=$META_DIR/gcroots

      mkdir -p "$STATE_DIR"

      # Same-filesystem guard. Hardlinks require source and destination
      # on the same fs. If someone moves /var/lib/nixling onto a
      # separate volume (NFS, separate LUKS dev, btrfs subvol, ...)
      # this whole approach breaks — fail loud rather than silently
      # degrade to copies.
      NIX_FS=$(stat -f -c '%T' /nix/store)
      STATE_FS=$(stat -f -c '%T' "$STATE_DIR")
      if [ "$NIX_FS" != "$STATE_FS" ]; then
        echo "nixling-store-sync: /nix/store ($NIX_FS) and $STATE_DIR ($STATE_FS) on different filesystems — cannot hardlink." >&2
        echo "  Move /var/lib/nixling back onto the same volume as /nix/store, or rebuild the per-VM store layout for cross-fs." >&2
        exit 3
      fi

      # If we are not already in a private mount namespace with
      # /nix/store unmasked, re-exec ourselves inside one. The outer
      # namespace is unaffected.
      if [ -z "''${NIXLING_STORE_SYNC_IN_NS:-}" ]; then
        export NIXLING_STORE_SYNC_IN_NS=1
        exec unshare --mount --propagation private \
          /bin/sh -c '
            umount -l /nix/store 2>/dev/null || true
            exec "$0" "$@"
          ' "$0" "$@"
      fi

      mkdir -p "$STORE_DIR" "$GEN_DIR" "$ROOT_DIR"

      # Serialize per-VM syncs across activation + CLI callers. The lock
      # stays held until process exit so generation allocation, current
      # pointer updates, and retention sweep are one critical section.
      LOCK_FILE=$META_DIR/store-sync.lock
      exec 9>"$LOCK_FILE"
      flock 9

      # Compute next generation number = max(existing dirs) + 1.
      MAX_GEN=0
      for d in "$GEN_DIR"/*; do
        [ -d "$d" ] || continue
        n=$(basename "$d")
        case "$n" in
          '''|*[!0-9]*) continue ;;
        esac
        if [ "$n" -gt "$MAX_GEN" ]; then MAX_GEN=$n; fi
      done
      NEXT_GEN=$((MAX_GEN + 1))

      # ---------- short-circuit: closure unchanged from current? ----------
      CURRENT_LINK=$META_DIR/current
      if [ -L "$CURRENT_LINK" ]; then
        CURRENT_GEN=$(basename "$(readlink "$CURRENT_LINK")")
        CUR_META=$GEN_DIR/$CURRENT_GEN/meta.json
        if [ -f "$CUR_META" ]; then
          CUR_TOP=$(jq -r .system "$CUR_META")
          CUR_SRC=$(jq -r '.source // ""' "$CUR_META")
          NEW_TOP=$(readlink "$GEN_SRC/system")
          NEW_SRC=$GEN_SRC
          if [ "$CUR_TOP" = "$NEW_TOP" ] && [ "$CUR_SRC" = "$NEW_SRC" ]; then
            marker="$STORE_DIR/.nixling-marker-$VM"
            missing=0
            while IFS= read -r path; do
              [ -n "$path" ] || continue
              base=''${path##*/}
              [ -e "$STORE_DIR/$base" ] || { missing=1; break; }
            done < "$GEN_SRC/store-paths"
            if [ "$missing" -eq 0 ] && [ -e "$marker" ]; then
              echo "nixling-store-sync: $VM already at generation $CURRENT_GEN ($NEW_TOP); nothing to do."
              exit 0
            fi
          fi
        fi
      fi

      echo "nixling-store-sync: $VM → generation $NEXT_GEN"
      NEW_TOP=$(readlink "$GEN_SRC/system")
      echo "  toplevel: $NEW_TOP"

      # ---------- hardlink-farm population ----------
      STAGE_DIR=$STORE_DIR.stage.$NEXT_GEN.$$
      mkdir -p "$STAGE_DIR"
      trap 'rm -rf "$STAGE_DIR"' EXIT

      NEW_COUNT=0
      SKIP_COUNT=0
      while IFS= read -r path; do
        [ -n "$path" ] || continue
        base=''${path##*/}
        if [ -e "$STORE_DIR/$base" ]; then
          SKIP_COUNT=$((SKIP_COUNT + 1))
          continue
        fi
        # `cp -al` does recursive hardlink. Works as long as source +
        # dest are on the same vfsmount (we unmounted /nix/store in
        # our private namespace above so this is now satisfied).
        cp -al "$path" "$STAGE_DIR/$base"
        NEW_COUNT=$((NEW_COUNT + 1))
      done < "$GEN_SRC/store-paths"

      # Move staged paths into store/ — directory rename within the
      # same parent is atomic per entry.
      if [ "$NEW_COUNT" -gt 0 ]; then
        for d in "$STAGE_DIR"/*; do
          [ -e "$d" ] || continue
          base=''${d##*/}
          if [ ! -e "$STORE_DIR/$base" ]; then
            mv -T "$d" "$STORE_DIR/$base"
          else
            rm -rf "$d"
          fi
        done
      fi
      rm -rf "$STAGE_DIR"
      trap - EXIT

      echo "  store-view/live: +$NEW_COUNT new, $SKIP_COUNT already present"

      # ---------- generation metadata ----------
      NEW_GEN_DIR=$GEN_DIR/$NEXT_GEN
      install -d -m 0755 "$NEW_GEN_DIR"
      install -m 0644 "$GEN_SRC/store-paths" "$NEW_GEN_DIR/store-paths"
      install -m 0644 "$GEN_SRC/db.dump"     "$NEW_GEN_DIR/db.dump"
      ln -sfT "$NEW_TOP" "$NEW_GEN_DIR/system"
      printf '{"closureHash":"toplevel:%s","nixlingVersion":"activation","activatedAt":"%s","vm":"%s","generationNumber":%d}\n' \
        "''${NEW_TOP##*/}" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$VM" "$NEXT_GEN" \
        > "$NEW_GEN_DIR/marker.json"
      printf '{"generation":%d,"timestamp":%d,"system":"%s","source":"%s"}\n' \
        "$NEXT_GEN" "$(date +%s)" "$NEW_TOP" "$GEN_SRC" \
        > "$NEW_GEN_DIR/meta.json"

      # GC root: pin the new generation against host nix-collect-garbage.
      ln -sfT "$NEW_TOP" "$ROOT_DIR/generation-$NEXT_GEN"
      # Register it as an indirect root so `nix-collect-garbage` honours it.
      install -d -m 0755 /nix/var/nix/gcroots/per-user/root/nixling 2>/dev/null || true
      ln -sfT "$ROOT_DIR/generation-$NEXT_GEN" \
        /nix/var/nix/gcroots/per-user/root/nixling/"$VM"-generation-"$NEXT_GEN" 2>/dev/null || true

      # ---------- atomic current flip ----------
      ln -sfT "generations/$NEXT_GEN" "$META_DIR/current.new"
      mv -T "$META_DIR/current.new" "$META_DIR/current"

      # ---------- retention ----------
      # Kept set = {NEXT_GEN} ∪ {running generation if VM up} ∪
      #            {most-recent prior generation if VM down}.
      KEEP=( "$NEXT_GEN" )

      RUNNING_TOP=""
      # The microvm hypervisor's cmdline includes the system path
      # /var/lib/nixling/vms/<vm>/booted points at the runner; the runner
      # path embeds the system toplevel. We grep for the system store
      # path via cmdline of any process associated with this VM.
      for pid in $(pgrep -f "microvm@$VM\\b|nixos-system-$VM-" 2>/dev/null || true); do
        cmd=$(tr '\0' ' ' < /proc/"$pid"/cmdline 2>/dev/null || true)
        # cmdline contains either the system toplevel path directly
        # (kernel + initrd args) or the runner; either way it embeds
        # the system hash. Extract any /nix/store/<hash>-nixos-system-<vm>-* path.
        match=$(echo "$cmd" | grep -oE "/nix/store/[a-z0-9]+-nixos-system-$VM-[^[:space:]]*" | head -1 || true)
        if [ -n "$match" ]; then
          # trim trailing path components (e.g. /init, /kernel) back to the system root
          base=$(echo "$match" | sed -E 's|^(/nix/store/[a-z0-9]+-nixos-system-'"$VM"'-[^/]+).*|\1|')
          RUNNING_TOP="$base"
          break
        fi
      done

      if [ -n "$RUNNING_TOP" ]; then
        # Find the generation whose system symlink points at $RUNNING_TOP.
        for d in "$GEN_DIR"/*; do
          [ -d "$d" ] || continue
          g=$(basename "$d")
          case "$g" in
            '''|*[!0-9]*) continue ;;
          esac
          target=$(readlink "$d/system" 2>/dev/null || true)
          if [ "$target" = "$RUNNING_TOP" ]; then
            KEEP+=( "$g" )
            break
          fi
        done
        echo "  VM running on $RUNNING_TOP; keeping its generation."
      else
        # VM is down: keep the most recent prior generation as a
        # rollback target.
        PRIOR=""
        for d in "$GEN_DIR"/*; do
          [ -d "$d" ] || continue
          g=$(basename "$d")
          case "$g" in
            '''|*[!0-9]*) continue ;;
          esac
          if [ "$g" != "$NEXT_GEN" ]; then
            if [ -z "$PRIOR" ] || [ "$g" -gt "$PRIOR" ]; then
              PRIOR=$g
            fi
          fi
        done
        if [ -n "$PRIOR" ]; then KEEP+=( "$PRIOR" ); fi
      fi

      # De-dup kept set.
      mapfile -t KEEP < <(printf '%s\n' "''${KEEP[@]}" | sort -u)
      echo "  keeping generations: ''${KEEP[*]}"

      # ---------- sweep store-view/live ----------
      # Union of store-paths across kept generations = paths we still need.
      KEEP_PATHS=$(mktemp)
      EXISTING_PATHS=$(mktemp)
      REMOVE_PATHS=$(mktemp)
      trap 'rm -f "$KEEP_PATHS" "$EXISTING_PATHS" "$REMOVE_PATHS"' EXIT
      for k in "''${KEEP[@]}"; do
        [ -f "$GEN_DIR/$k/store-paths" ] && cat "$GEN_DIR/$k/store-paths" >> "$KEEP_PATHS"
      done
      sed -n 's|.*/||p' "$KEEP_PATHS" | sort -u > "$KEEP_PATHS.basenames"
      mv "$KEEP_PATHS.basenames" "$KEEP_PATHS"

      find "$STORE_DIR" -mindepth 1 -maxdepth 1 -printf '%f\n' \
        | ${pkgs.gawk}/bin/awk -v marker=".nixling-marker-$VM" '$0 != marker { print }' \
        | sort -u > "$EXISTING_PATHS"
      comm -23 "$EXISTING_PATHS" "$KEEP_PATHS" > "$REMOVE_PATHS"

      REMOVED=0
      while IFS= read -r base; do
        [ -n "$base" ] || continue
        case "$base" in
          .nixling-marker-*|live.stage.*) continue ;;
        esac
        rm -rf "''${STORE_DIR:?}/$base"
        REMOVED=$((REMOVED + 1))
      done < "$REMOVE_PATHS"
      rm -f "$EXISTING_PATHS" "$REMOVE_PATHS"
      trap 'rm -f "$KEEP_PATHS"' EXIT
      echo "  store-view/live: -$REMOVED pruned"

      # Prune unkept generations + their gcroots after the live pool is swept.
      for d in "$GEN_DIR"/*; do
        [ -d "$d" ] || continue
        g=$(basename "$d")
        case "$g" in
          '''|*[!0-9]*) continue ;;
        esac
        keep=0
        for k in "''${KEEP[@]}"; do
          if [ "$g" = "$k" ]; then keep=1; break; fi
        done
        if [ "$keep" -eq 0 ]; then
          echo "  pruning generation $g"
          rm -rf "''${GEN_DIR:?}/$g"
          rm -f "''${ROOT_DIR:?}/generation-$g"
          rm -f "/nix/var/nix/gcroots/per-user/root/nixling/$VM-generation-$g" 2>/dev/null || true
        fi
      done

      # Permissions: align store-view directory inodes with
      # the daemon ownership matrix (`nixlingd:users 0755`). The owner
      # (`nixlingd`) can mutate directory entries during StoreSync; the
      # broad `users` group gets read/execute only so local users cannot
      # unlink or replace entries in the guest store view or metadata.
      #
      # Only chmod AND chown the directory inodes that
      # nixling creates (the per-VM /var/lib/nixling/vms/<vm>/store-view tree).
      # Recursive chmod or chown on the files would change the
      # hardlinked /nix/store inodes too, violating Nix store
      # immutability — a virtiofsd RCE that escapes the per-VM bind
      # could then locate the same inodes via name_to_handle_at and
      # have writable+exec perms (or unexpected group ownership) on
      # them. File inodes retain their upstream Nix store ownership
      # (root:root) and modes (0555 for executables, 0444 for data).
      find "$META_DIR" -type d -exec chown nixlingd:users {} + 2>/dev/null || true
      find "$META_DIR" -type d -exec chmod 0755 {} + 2>/dev/null || true

      # Plant the per-VM marker (C1a). The microvm-virtiofsd@<vm>.service
      # drop-in ExecStartPre tests for this exact path before allowing
      # the unit to start, so a hand-crafted /var/lib/nixling/vms/<vm>/store-view/live/
      # populated by anything other than nixling-store-sync cannot be
      # served by virtiofsd. Mode 0444 so the unprivileged virtiofsd
      # user (nl-virtiofs-<vm>) can read it but not modify it.
      MARKER="$STORE_DIR/.nixling-marker-$VM"
      : > "$MARKER"
      chmod 0444 "$MARKER"

      # Bump current's mtime so the guest's path-trigger (in base.nix)
      # fires to re-load db.dump.
      touch "$META_DIR/current" 2>/dev/null || true

      echo "nixling-store-sync: $VM done (generation $NEXT_GEN active)."
    '';
  };

  # Map VM name → its generation derivation outPath. Used by the
  # activation script + the systemd template.
  vmGenPaths = lib.mapAttrs (name: _: vmGenerationOf name) enabledVms;

in

{
  options.nixling.store = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Materialise a per-VM hardlink-farm `/nix/store` for every
        nixling microVM. When false, microvm.nix's default behaviour
        (share host's full `/nix/store`) is restored. The toggle is
        global; per-VM opt-out is not supported.
      '';
    };

    stateDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/nixling/vms";
      description = ''
        Root of every VM's per-VM nix store. Must be on the same
        filesystem as `/nix/store` (hardlinks require this). The
        sync helper enforces this at runtime.
      '';
    };

    # ---------------------------------------------------------------------------
    # the previously-exposed internal
    # options `nixling.store.package` and `nixling.store.generations`
    # were retired together with the bash CLI. The only consumer was
    # `cli.nix` (the `nixling switch` codepath); store.nix itself
    # uses the `nixlingStoreSync` derivation and `vmGenPaths`
    # let-bindings directly. Both bindings remain available for the
    # daemon-native StoreSync surface, which will plumb them through
    # bundle.nix / processes-json.nix instead of re-exposing a
    # readOnly NixOS option (issue #6 — see tests/static.sh trio lint).
    # ---------------------------------------------------------------------------
  };

  config = lib.mkIf cfg.store.enable {
    # Expose the helper on the host so it can be invoked manually or
    # from the nixling CLI without going through systemd.
    environment.systemPackages = [ nixlingStoreSync spectrumCh ];

    # Force every VM's nix-store share to point at its per-VM hardlink
    # farm instead of the host's full /nix/store. microvm.nix's
    # mounts.nix REQUIRES one share with literal `source =
    # "/nix/store"` so it can wire writableStoreOverlay correctly; we
    # keep that literal here and override the actual served path at
    # runtime with a BindReadOnlyPaths drop-in on
    # microvm-virtiofsd@<vm> (further down).
    #
    # `microvm.writableStoreOverlay = "/nix/.rw-store"` is REQUIRED, not
    # optional. Without it the guest mounts the virtiofs share read-only
    # at /nix/store, which makes `nix-env --profile … --set` (the first
    # action `nixling switch/boot` runs in the guest) fail with
    #   error: creating directory "/nix/store/.links": Read-only file system
    # because nix-env's optimised-store and validity-registration paths
    # both want to write under /nix/store. The overlay layers a tmpfs
    # writable upper over the read-only lower, so those writes go to
    # tmpfs and the activation step completes. Overlay contents are
    # ephemeral (wiped each VM reboot) which is fine — anything the
    # host needs to be persistent lives in the per-VM hardlink farm and
    # is rebuilt on the next `nixling-store-sync`.
    #
    # Also add a tiny second share for store-meta (db.dump + generation
    # info), mounted in the guest at /run/nixling-store-meta.
    # Per-VM nix-store + meta + host-keys shares are injected by
    # host.nix's composeVm pass directly (see
    # `nixos-modules/host.nix` composedConfig). store.nix used to
    # write `nixling.vms = lib.mapAttrs... { config.microvm.shares
    # = ...; }` here, but reading cfg.vms while writing to
    # nixling.vms causes module-system infinite recursion. The
    # shares injection moved into host.nix where the per-VM
    # composedConfig has direct access to the consumer's `vm`
    # struct without needing to round-trip through cfg.vms.

    # microvm-virtiofsd@<vm> systemd drop-ins REMOVED.
    # The upstream `microvm-virtiofsd@.service` template doesn't
    # exist anymore (microvm.nix flake input dropped per ADR 0018);
    # the broker's Virtiofsd SpawnRunner role owns virtiofsd
    # supervision end-to-end, including the marker-file gate
    # (now enforced via `BundleResolver::ResolvedRunnerIntent`'s
    # pre-spawn validation in `nixling-priv-broker::runtime`),
    # the hardened exec wrapper (Rust generator output in
    # `nixling-host::virtiofsd_argv`), and the CapabilityBoundingSet
    # (broker spawn-time `set_capabilities` in `nixling-priv-broker::sys`).
    # The drop-ins block that lived here would target a unit that
    # no longer exists; deleted.
    systemd.services = {

    # nixling-vfsd-watchdog@ template + per-VM enable units
    # + timer template RETIRED per ADR 0018. The vhost-user-fs wedge
    # detection moved into the broker's Virtiofsd `SpawnRunner` role
    # supervisor: the broker holds the pidfd via clone3(CLONE_PIDFD)
    # and polls via pidfd_send_signal(SIGCONT,0) + cgroup.events
    # population probe at the same 60s cadence; wedge surfaces via
    # the typed `runner-wedged` OpAuditRecord rather than the
    # journal-scan + systemctl-stop pair this template used.
    #
    # The script body, per-VM enabling units, and timer template
    # that lived here are deleted; the trailing `{}` keeps the
    # outer attribute-union shape stable for any future per-VM
    # service additions.
    };

    # retired
    #   - systemd.services."nixling-vfsd-watchdog@"
    #   - systemd.services."nixling-vfsd-watchdog-<vm>-enable" (per-VM)
    #   - systemd.timers."nixling-vfsd-watchdog@"
    # All three replaced by the broker Virtiofsd SpawnRunner role's
    # pidfd-based wedge detection (per ADR 0018).

    # ---------------------------------------------------------------------------
    # Host activation hook.
    # On every `nixos-rebuild switch`, drop the per-VM next-generation
    # pointer into /run/nixling/<vm>/next-generation. The daemon-native
    # Rust StoreSync broker op is the canonical writer for store-view;
    # activation must not build/sweep/activate per-VM store closures.
    #
    # /run/nixling is created by
    # host-daemon.nix tmpfiles (nixlingd:nixling 0750) under
    # daemonExperimental, or host.nix tmpfiles (root:nixling
    # 0775) without it. This activation hook MUST NOT touch the parent
    # `/run/nixling` directory — only per-VM leaves.
    # ---------------------------------------------------------------------------
    system.activationScripts.nixlingStoreSync = lib.stringAfter [ "specialfs" "users" ] ''
      set -u
      ${lib.concatStringsSep "\n" (lib.mapAttrsToList
        (name: gen: ''
          install -d -m 0755 /run/nixling/${name}
          ln -sfT ${gen} /run/nixling/${name}/next-generation
        '')
        vmGenPaths)}
    '';

    # ---------------------------------------------------------------------------
    # �� vhost-user reconnect resilience.
    #
    # Background: when microvm-virtiofsd@<vm>.service is restarted while
    # microvm@<vm>.service is active, the cloud-hypervisor process keeps
    # running but its vhost-user connections to virtiofsd die. CH tries
    # to reconnect for 60s, gives up, marks the virtio-fs devices as
    # NEEDS_RESET and stops processing their queues. The guest kernel
    # then blocks indefinitely on any read of /nix/.ro-store -- vcpu
    # spinning at 100%, no SSH, no console. Observed on a live graphics
    # VM during the C1a/C1b iterations.
    #
    # microvm.nix sets `X-RestartIfChanged=false` on microvm@<vm> so a
    # nixos-rebuild does NOT bounce running VMs on every switch. Good
    # for normal config changes; broken for changes that bounce
    # virtiofsd, because then microvm@ keeps running but its CH is sick.
    #
    # Two layers of defense
    #
    #   1. Activation-time hint: if virtiofsd's drop-in for a given VM
    #      changed AND microvm@<vm> is currently active, log a clear
    #      warning. Do NOT auto-stop the VM (would disrupt the user mid-
    #      task); just tell her.
    #
    #   2. Periodic healthcheck (`nixling-vfsd-watchdog@<vm>.timer` /
    #      `.service`): every 60s, check the CH journal for the
    #      NEEDS_RESET pattern. If present, kill microvm@<vm> so the
    #      autorestart (workload VMs) or the operator's manual `nixling up`
    #      (graphics VMs) replaces the wedged CH with a healthy one.
    # ---------------------------------------------------------------------------

    system.activationScripts.nixlingVfsdRestartHint = lib.stringAfter [ "etc" ] ''
      set -u
      ${lib.concatStringsSep "\n" (lib.mapAttrsToList
        (name: _: ''
          if /run/current-system/sw/bin/systemctl is-active --quiet microvm@${name}.service 2>/dev/null; then
            # systemctl show ExecStart --value returns
            # "{ path=/nix/store/...; argv[]= ...;... }"; extract just
            # the /nix/store path so we can compare against the new
            # ExecStart line in overrides.conf, which is a raw path.
            cur_path=$(/run/current-system/sw/bin/systemctl show microvm-virtiofsd@${name}.service -p ExecStart --value 2>/dev/null \
                       | ${pkgs.gnugrep}/bin/grep -oE 'path=[^ ;}]+' \
                       | ${pkgs.gnused}/bin/sed 's|^path=||' \
                       | head -1)
            new_path=$(${pkgs.gnugrep}/bin/grep -E '^ExecStart=/' /etc/systemd/system/microvm-virtiofsd@${name}.service.d/overrides.conf 2>/dev/null \
                       | tail -1 \
                       | ${pkgs.gnused}/bin/sed 's|^ExecStart=||')
            if [ -n "$cur_path" ] && [ -n "$new_path" ] && [ "$cur_path" != "$new_path" ]; then
              echo "nixling: WARNING — microvm-virtiofsd@${name}.service ExecStart changed (was: $cur_path; now: $new_path) AND microvm@${name}.service is active."
              echo "nixling:   The activation will restart virtiofsd, but cloud-hypervisor will lose its vhost-user backend."
              echo "nixling:   You must restart the VM after this rebuild: sudo systemctl stop microvm@${name}.service && nixling up ${name}"
              echo "nixling:   (the nixling-vfsd-watchdog@${name}.timer will detect the wedge and stop microvm@ within ~60s.)"
            fi
          fi
        '')
        enabledVms)}
    '';
  };
}
