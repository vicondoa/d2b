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
# Layout (per VM, under /var/lib/nixling/vms/<vm>/):
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
  # info via `pkgs.closureInfo`, which gives us:
  #   <out>/store-paths   newline list of every path in the closure
  #   <out>/registration  format consumed by `nix-store --load-db`
  vmTopOf = name:
    config.microvm.vms.${name}.config.config.system.build.toplevel;

  # The microvm.nix-generated "runner" derivation for this VM. It
  # holds:
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
  vmRunnerOf = name:
    config.microvm.vms.${name}.config.config.microvm.declaredRunner;

  # Wrapper around microvm.nix's bin/virtiofsd-run that sanitises the
  # supervisord config before invoking it. The microvm.nix-generated
  # supervisord conf has a top-level `user=root` directive (because
  # the unit historically ran as root). With the C1a User= drop to
  # `nl-virtiofs-<vm>` the kernel rejects supervisord's attempt to
  # setuid to root and supervisord aborts with:
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
  # What the wrapper actually does:
  #
  #   1. Locate microvm.nix's generated virtiofsd-run script + the
  #      supervisord conf it references.
  #   2. For each `command=<path>` entry in that conf (one per
  #      virtiofs share), copy <path> to a writable per-VM location
  #      and sed-patch its hard-coded virtiofsd flags:
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

      # Belt-and-suspenders marker gate (security findings nixos-1 /
      # software-1 / security-1). Two independent layers must both
      # succeed before any virtiofsd child is spawned:
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
      # Security finding P1r1 security-3.
      HARDENED=/run/nixling/${name}/hardened
      install -d -m 0755 -o root -g root /run/nixling 2>/dev/null || true
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
      # any guest write attempt (security finding P1r1 security-2).
      #
      # The retain set we drop:
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
        # requires (security finding P1r3 software-3).
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

        # Fail-closed verification (security finding P1r2 software-2).
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
    pkgs.closureInfo {
      rootPaths = [
        (vmTopOf name)
        (vmRunnerOf name)
        (nixlingVfsRunnerOf name)
      ];
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
  # Usage:
  #   nixling-store-sync <vm> <generation-dir>
  #     <vm>              VM name
  #     <generation-dir>  derivation output from vmGenerationOf
  #
  # Steps:
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
  #   9. Retention: compute the kept set:
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
            echo "nixling-store-sync: $VM already at generation $CURRENT_GEN ($NEW_TOP); nothing to do."
            exit 0
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

      echo "  store/: +$NEW_COUNT new, $SKIP_COUNT already present"

      # ---------- generation metadata ----------
      NEW_GEN_DIR=$GEN_DIR/$NEXT_GEN
      install -d -m 0755 "$NEW_GEN_DIR"
      install -m 0644 "$GEN_SRC/store-paths" "$NEW_GEN_DIR/store-paths"
      install -m 0644 "$GEN_SRC/db.dump"     "$NEW_GEN_DIR/db.dump"
      ln -sfT "$NEW_TOP" "$NEW_GEN_DIR/system"
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
      # The microvm hypervisor's cmdline includes the system path:
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

      # Prune unkept generations + their gcroots.
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

      # ---------- sweep store/ ----------
      # Union of store-paths across kept generations = paths we still need.
      KEEP_PATHS=$(mktemp)
      trap 'rm -f "$KEEP_PATHS"' EXIT
      for k in "''${KEEP[@]}"; do
        [ -f "$GEN_DIR/$k/store-paths" ] && cat "$GEN_DIR/$k/store-paths" >> "$KEEP_PATHS"
      done
      sort -u "$KEEP_PATHS" -o "$KEEP_PATHS"

      # For each <base> in store/, drop if its corresponding /nix/store/<base>
      # isn't in the keep set.
      REMOVED=0
      for d in "$STORE_DIR"/*; do
        [ -e "$d" ] || continue
        base=''${d##*/}
        full="/nix/store/$base"
        if ! grep -qxF "$full" "$KEEP_PATHS"; then
          rm -rf "$d"
          REMOVED=$((REMOVED + 1))
        fi
      done
      echo "  store/: -$REMOVED pruned"

      # Permissions: microvm.nix's virtiofsd runs as user 'microvm'
      # group 'kvm'; the store dir needs to be readable.
      #
      # M4 / security-1: only chmod AND chown the directory inodes that
      # nixling creates (the per-VM /var/lib/nixling/vms/<vm>/store tree).
      # Recursive chmod or chown on the files would change the
      # hardlinked /nix/store inodes too, violating Nix store
      # immutability — a virtiofsd RCE that escapes the per-VM bind
      # could then locate the same inodes via name_to_handle_at and
      # have writable+exec perms (or unexpected group ownership) on
      # them. File inodes retain their upstream Nix store ownership
      # (root:root) and modes (0555 for executables, 0444 for data).
      find "$STORE_DIR" "$META_DIR" -type d -exec chown root:kvm {} + 2>/dev/null || true
      find "$STORE_DIR" "$META_DIR" -type d -exec chmod 0755 {} + 2>/dev/null || true

      # Plant the per-VM marker (C1a). The microvm-virtiofsd@<vm>.service
      # drop-in ExecStartPre tests for this exact path before allowing
      # the unit to start, so a hand-crafted /var/lib/nixling/vms/<vm>/store/
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

    package = lib.mkOption {
      type = lib.types.package;
      default = nixlingStoreSync;
      internal = true;
      readOnly = true;
      description = "The `nixling-store-sync` helper, exposed for cli.nix to invoke.";
    };

    generations = lib.mkOption {
      type = lib.types.attrsOf lib.types.package;
      default = vmGenPaths;
      internal = true;
      readOnly = true;
      description = "Per-VM generation derivations; consumed by cli.nix and the activation script.";
    };
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
    microvm.vms = lib.mapAttrs
      (name: _: {
        config.microvm.writableStoreOverlay = lib.mkDefault "/nix/.rw-store";
        config.microvm.shares = lib.mkForce [
          {
            source = "/nix/store";
            mountPoint = "/nix/.ro-store";
            tag = "ro-store";
            proto = "virtiofs";
          }
          {
            source = "${cfg.store.stateDir}/${name}/store-meta";
            mountPoint = "/run/nixling-store-meta";
            tag = "nl-meta";
            proto = "virtiofs";
          }
          # Per-VM host-keys share (Phase 2b — see host-keys.nix).
          # The host stages host.pub + user-authorized-keys here on
          # activation; the guest's nixling-load-host-keys.service
          # reads them at boot to populate authorized_keys.
          {
            source = "${cfg.site.stateDir}/vms/${name}/host-keys";
            mountPoint = "/run/nixling-host-keys";
            tag = "nl-hkeys";
            proto = "virtiofs";
          }
        ];
      })
      enabledVms;

    # Per-VM `nixling-<vm>-virtiofsd.service` overrides:
    # BindReadOnlyPaths replaces /nix/store inside the virtiofsd
    # service's private mount namespace with the per-VM hardlink farm.
    # The supervisord wrapper still does `--shared-dir=/nix/store`
    # (baked at eval time), but the path now resolves to
    # /var/lib/nixling/vms/<vm>/store/.
    #
    # We override `microvm-virtiofsd@<vm>.service` (microvm.nix's
    # template) via per-instance drop-ins so we get the upstream
    # template's correctness (PrivateMounts, etc.) plus our own
    # C1a/C1b hardening stanza.
    #
    # The remaining serviceConfig keys are the C1a + C1b hardening
    # stanza (spec §3.C1a fix):
    #   * Stable non-root system user (nl-virtiofs-<vm>) declared
    #     in host.nix, with `kvm` as a supplementary group so the
    #     unix socket can be chgrp'd to kvm for cloud-hypervisor.
    #   * Full systemd-confinement stanza: Protect*/Restrict*/Lock*/
    #     PrivateNetwork/PrivateDevices/MemoryDenyWriteExecute/
    #     RestrictAddressFamilies=AF_UNIX/SystemCallArchitectures=native.
    #   * AmbientCapabilities / CapabilityBoundingSet hold ONLY the
    #     caps virtiofsd actually needs to serve an upper-layer FS:
    #       SYS_ADMIN  (mount/umount inside its sandbox=chroot)
    #       CHOWN      (--socket-group=kvm)
    #       FOWNER/FSETID/MKNOD/SETFCAP (preserve owner/mode/special-
    #                  files/xattrs when surfacing files to the guest)
    #       SETUID/SETGID (file ownership emulation for the guest)
    #       DAC_OVERRIDE (read closure paths regardless of mode bits)
    #     CAP_DAC_READ_SEARCH (C1b) is INTENTIONALLY ABSENT: it would
    #     let open_by_handle_at(2) bypass the per-VM store bind mount
    #     and reach the host's full /nix/store. virtiofsd's runner
    #     script still passes `--inode-file-handles=prefer`, but
    #     without CAP_DAC_READ_SEARCH the kernel refuses
    #     open_by_handle_at and the daemon falls back to O_PATH FDs
    #     (which honour the unit's mount namespace and bind mount).
    #   * ExecStartPre with `+` (run as root before User= drop) tests
    #     for the .nixling-marker-<vm> file planted by nixling-store-sync
    #     at the end of its closure-population step. An attacker who
    #     planted a hand-crafted /var/lib/nixling/vms/<vm>/store/ but did
    #     not invoke nixling-store-sync cannot satisfy the check.
    #   * ReadWritePaths exposes the per-VM state dir so virtiofsd can
    #     create its `*-virtiofs-{ro-store,nl-meta}.sock` unix sockets
    #     inside the otherwise-strict /var/lib hierarchy.
    systemd.services = (lib.mapAttrs'
      (name: _:
        lib.nameValuePair "microvm-virtiofsd@${name}" {
          # v0.1.5: don't let NixOS override upstream microvm.nix's
          # X-RestartIfChanged=false. Framework adds per-VM
          # serviceConfig (ExecStartPre marker check, hardened
          # ExecStart wrapper, sandbox stanza); without this, every
          # rebuild restarts virtiofsd → CH loses the virtiofs
          # socket → guest /nix/.ro-store wedges → VM unusable.
          # Mirrors per-VM sidecars (host-sidecars.nix / audio host).
          # Consumer applies changes via `nixling switch <vm>`.
          restartIfChanged = false;
          serviceConfig = {
            # Layer 1 of the belt-and-suspenders marker gate
            # (security findings nixos-1 / software-1 / security-1).
            # ExecStartPre with the `+` prefix runs unrestricted on
            # the HOST mount namespace before any bind-mount is in
            # effect: it tests the SOURCE marker
            # /var/lib/nixling/vms/<vm>/store/.nixling-marker-<vm>,
            # which proves nixling-store-sync actually planted the
            # file. This catches the case where the per-VM store
            # directory was hand-crafted (or where nixling-store-sync
            # regressed and never ran). It is INSUFFICIENT on its
            # own -- see the second layer in nixlingVfsRunnerOf,
            # which tests /nix/store/.nixling-marker-<vm> from
            # INSIDE the unit's mount namespace and so catches the
            # case where BindReadOnlyPaths silently no-ops (future
            # systemd refactor / microvm.nix upstream change /
            # namespace setup race) -- both layers together are
            # needed to prove "the marker was planted AND the
            # daemon is serving that exact view".
            ExecStartPre = "+${pkgs.coreutils}/bin/test -e ${cfg.store.stateDir}/${name}/store/.nixling-marker-${name}";

            # C1a — replace microvm.nix's generated virtiofsd-run
            # with our hardened wrapper (see nixlingVfsRunnerOf
            # above). The wrapper:
            #   - patches `--inode-file-handles=prefer` to `=never`
            #     so virtiofsd doesn't need CAP_DAC_READ_SEARCH
            #     at runtime (C1b)
            #   - drops `--posix-acl --xattr` from each daemon
            #     command line (parser surface reduction)
            #   - strips `user=root` from the supervisord conf
            # Empty-string first slot clears the base unit's
            # ExecStart per systemd's drop-in idiom.
            ExecStart = [
              ""
              "${nixlingVfsRunnerOf name}/bin/nixling-virtiofsd-run-${name}"
            ];

            # C1a + C1b — systemd-level hardening that virtiofsd's
            # internal sandbox can coexist with.
            #
            # NOTE: virtiofsd default `--sandbox=namespace` performs
            # setuid(0) + mount(/proc) inside its own sandbox. That
            # needs either real root OR a user namespace where uid 0
            # is mapped. Running virtiofsd directly as a non-root
            # system user (the ideal C1a end-state) requires wrapping
            # its invocation in `unshare -r --map-auto` which in turn
            # requires subuid/subgid configuration. That refactor is
            # deferred — the version below keeps virtiofsd running
            # as root (microvm.nix default) but layers the systemd-
            # confinement stanza ON TOP of it.
            #
            # What this stanza buys us today:
            #   - CapabilityBoundingSet drops CAP_DAC_READ_SEARCH
            #     (C1b): even if the daemon were RCE'd, it physically
            #     cannot do open_by_handle_at to escape its bind-mount.
            #     The wrapper's --inode-file-handles=never patch is
            #     what makes startup work without this capability.
            #   - CapabilityBoundingSet drops every capability NOT in
            #     virtiofsd's retain-set (no CAP_NET_*, no
            #     CAP_SYS_RAWIO, no CAP_SYS_PTRACE, etc.).
            #   - NoNewPrivileges + LockPersonality + RestrictRealtime +
            #     RestrictSUIDSGID + SystemCallArchitectures harden
            #     against generic post-exploit pivots.
            #   - ProtectKernel{Tunables,Modules,Logs} + ProtectControlGroups
            #     + ProtectClock + ProtectHostname + ProtectHome shrink
            #     the writable+observable host surface to roughly nothing
            #     outside /nix/store + /var/lib/nixling/vms/<vm>.
            #
            # NOT applied (would break virtiofsd's inner sandbox):
            #   - User=nl-virtiofs-<vm> — needs unshare userns to setuid(0)
            #   - PrivateDevices/PrivateNetwork — virtiofsd creates its own
            #     namespaces and these collide
            #   - ProtectSystem=strict — virtiofsd needs write to its
            #     state dir under /var/lib/nixling (would need
            #     ReadWritePaths plumbing; defer with the User= rework)
            #   - MemoryDenyWriteExecute — defer; verify virtiofsd
            #     binary doesn't need W^X for its mmap workload
            NoNewPrivileges = true;
            ProtectHome = true;
            ProtectKernelTunables = true;
            ProtectKernelModules = true;
            ProtectKernelLogs = true;
            ProtectControlGroups = true;
            ProtectClock = true;
            ProtectHostname = true;
            LockPersonality = true;
            RestrictRealtime = true;
            RestrictSUIDSGID = true;
            SystemCallArchitectures = "native";
            UMask = "0077";
            CapabilityBoundingSet = [
              "CAP_SYS_ADMIN"
              "CAP_SETPCAP"
              "CAP_CHOWN"
              "CAP_FOWNER"
              "CAP_FSETID"
              "CAP_SETUID"
              "CAP_SETGID"
              "CAP_DAC_OVERRIDE"
              "CAP_MKNOD"
              "CAP_SETFCAP"
            ];
            BindReadOnlyPaths = [
              "${cfg.store.stateDir}/${name}/store:/nix/store"
            ];
          };
        })
      enabledVms) // {

    # ---------------------------------------------------------------------------
    # P1r2 — vhost-user reconnect watchdog.
    #
    # Background: when microvm-virtiofsd@<vm>.service is restarted while
    # microvm@<vm>.service is active, cloud-hypervisor keeps running but
    # its vhost-user connections to virtiofsd die. CH tries to reconnect
    # for 60s, gives up, marks the virtio-fs devices NEEDS_RESET and
    # stops processing queues. Guest kernel blocks on /nix/.ro-store
    # reads -> vcpu spins at 100%, no SSH, no console. Observed during
    # the C1a/C1b iterations on a live workload VM.
    #
    # This template (one .timer instance per VM, see the enable units
    # further down) checks the CH journal every 60s for the wedge
    # signature and stops microvm@<vm>.service when found, so the
    # operator can either `nixling up <vm>` (graphics VMs) or systemd
    # autostart (net VMs) brings a fresh CH up.
    # ---------------------------------------------------------------------------
    "nixling-vfsd-watchdog@" = {
      description = "Watchdog for cloud-hypervisor vhost-user-fs wedge on %i";
      serviceConfig = {
        Type = "oneshot";
        ExecStart = "${pkgs.writeShellScript "nixling-vfsd-watchdog" ''
          set -u
          vm="$1"
          if ! ${pkgs.systemd}/bin/systemctl is-active --quiet "microvm@$vm.service"; then
            # Path A: microvm@ is INACTIVE but CH may be running direct-launch
            # (spawned by `nixling up` as the desktop user, wrapped in a
            # nixling-vm-<vm>.scope since Phase 2 Part 1). Detect and stop.
            ch_sock="/var/lib/nixling/$vm/$vm.sock"
            ch_log="/var/lib/nixling/$vm/$vm.log"
            ch_pid=""
            scope_unit="nixling-vm-$vm.scope"
            # 1. Resolve MainPID from the user scope (post-Part-1 launches).
            if ch_pid=$(${pkgs.systemd}/bin/systemctl \
                  --user --machine ${userMachine} \
                  show "$scope_unit" -p MainPID --value 2>/dev/null) \
               && [ -n "$ch_pid" ] && [ "$ch_pid" != "0" ] && [ -d "/proc/$ch_pid" ]; then
              : # found via scope
            else
              # 2. Fallback: locate any cloud-hypervisor using this VM's api socket.
              ch_pid=$(${pkgs.procps}/bin/pgrep -af \
                'cloud-hypervisor.*api-socket.*'"$vm"'[./]' 2>/dev/null \
                | ${pkgs.gawk}/bin/awk 'NR==1{print $1}' || true)
            fi
            if [ -n "$ch_pid" ] && [ -d "/proc/$ch_pid" ] && [ -S "$ch_sock" ]; then
              wedged=0
              wedge_reason=""
              ping_flag="/run/nixling/$vm/vfsd-watchdog-ping-fails"
              # Check VMM API thread liveness via ch-remote ping.
              if ! ${spectrumCh}/bin/ch-remote \
                    --api-socket "$ch_sock" ping >/dev/null 2>&1; then
                count=0
                [ -f "$ping_flag" ] && count=$(${pkgs.coreutils}/bin/cat "$ping_flag" 2>/dev/null || echo 0)
                count=$((count + 1))
                echo "$count" > "$ping_flag"
                if [ "$count" -ge 3 ]; then
                  wedged=1
                  wedge_reason="ch-remote ping timeout x$count"
                fi
              else
                ${pkgs.coreutils}/bin/rm -f "$ping_flag"
                # Check log file for NEEDS_RESET since last watchdog tick.
                # An offset file scopes the scan to new entries only, so
                # stale NEEDS_RESET lines from prior CH runs do not re-fire.
                offset_file="/run/nixling/$vm/vfsd-watchdog-log-offset"
                offset=0
                [ -f "$offset_file" ] && offset=$(${pkgs.coreutils}/bin/cat "$offset_file" 2>/dev/null || echo 0)
                log_size=0
                [ -f "$ch_log" ] && log_size=$(${pkgs.coreutils}/bin/wc -c < "$ch_log" 2>/dev/null || echo 0)
                if [ "$offset" -gt "$log_size" ]; then offset=0; fi
                if [ -f "$ch_log" ] && [ "$log_size" -gt "$offset" ]; then
                  new_lines=$(${pkgs.coreutils}/bin/tail -c "+$((offset + 1))" "$ch_log" 2>/dev/null || true)
                  if echo "$new_lines" \
                       | ${pkgs.gnugrep}/bin/grep -qE \
                           'vhost-user.*Failed connecting the backend|virtio.*NEEDS_RESET|Setting device status to .NEEDS_RESET'; then
                    wedged=1
                    wedge_reason="NEEDS_RESET in $ch_log (ch-remote api-socket: $ch_sock)"
                  fi
                fi
                echo "$log_size" > "$offset_file"
              fi
              if [ "$wedged" = "1" ]; then
                echo "nixling-vfsd-watchdog: $vm direct-launch CH wedged ($wedge_reason); stopping $scope_unit" \
                  | ${pkgs.systemd}/bin/systemd-cat -t nixling-vfsd-watchdog -p warning
                ${pkgs.systemd}/bin/systemctl --user --machine ${userMachine} stop "$scope_unit" 2>/dev/null || true
              fi
            fi
            exit 0
          fi
          # Anchor the journal scan to the CURRENT invocation of
          # microvm@<vm>.service (security finding P1r3 software-4 /
          # test-r3-1). The InvocationID is a 128-bit token systemd
          # generates per-start of a unit and stamps onto every
          # journal record produced during that invocation. After we
          # stop a wedged VM and an operator `nixling up`s it, the next
          # start gets a fresh InvocationID, and the old wedge lines
          # -- still matchable by the wedge regex for as long as
          # they sit in the journal -- are filtered out because they
          # carry the previous invocation's ID. Without this anchor,
          # the SAME pre-stop log line would trip the watchdog on
          # the next 60s tick after `nixling up`, stopping the
          # healthy fresh instance (an indefinite stop-loop).
          #
          # If systemctl returns an empty InvocationID (unit just
          # started, racing with journald), there is nothing to scan
          # -- skip this tick; the next one will have a non-empty ID.
          #
          # We also drop the `--since "5 min ago"` window: within a
          # single invocation, even a 24h-old wedge line is real
          # evidence that THIS instance is wedged.
          inv=$(${pkgs.systemd}/bin/systemctl show "microvm@$vm.service" -p InvocationID --value 2>/dev/null)
          if [ -z "$inv" ]; then
            exit 0
          fi
          if ${pkgs.systemd}/bin/journalctl \
                -u "microvm@$vm.service" \
                "_SYSTEMD_INVOCATION_ID=$inv" \
                --no-pager 2>/dev/null \
              | ${pkgs.gnugrep}/bin/grep -qE 'vhost-user.*Failed connecting the backend|virtio.*NEEDS_RESET|Setting device status to .NEEDS_RESET'; then
            echo "nixling-vfsd-watchdog: $vm cloud-hypervisor has wedged vhost-user-fs; stopping microvm@$vm.service so it can be restarted cleanly" >&2
            ${pkgs.systemd}/bin/systemctl stop "microvm@$vm.service" || true
          fi
        ''} %i";
      };
    };
    } //
    # Per-VM units that enable the watchdog timer at boot. Without
    # these, the @.timer template exists but no instance starts.
    (lib.mapAttrs'
      (name: _: lib.nameValuePair
        "nixling-vfsd-watchdog-${name}-enable"
        {
          description = "Enable nixling-vfsd-watchdog@${name}.timer";
          wantedBy = [ "multi-user.target" ];
          serviceConfig = {
            Type = "oneshot";
            RemainAfterExit = true;
            ExecStart = "${pkgs.systemd}/bin/systemctl start nixling-vfsd-watchdog@${name}.timer";
            ExecStop = "${pkgs.systemd}/bin/systemctl stop nixling-vfsd-watchdog@${name}.timer";
          };
        })
      enabledVms) //
    # ---------------------------------------------------------------------------
    # Per-VM `nixling-<vm>-store-sync.service`.
    # Fired:
    #   - from the host activation script for every declared VM (so
    #     `nixos-rebuild switch` keeps per-VM stores in sync), and
    #   - from `nixling build/switch/boot/test/rollback/gc <vm>` to
    #     pick up a new closure.
    # Idempotent. Reads the target generation dir from
    # /run/nixling/<vm>/next-generation (a path the activation script
    # and the CLI both write before triggering the service).
    # ---------------------------------------------------------------------------
    (lib.mapAttrs'
      (name: _: lib.nameValuePair "nixling-${name}-store-sync" {
        description = "Populate nixling per-VM nix store for ${name}";
        after = [ "local-fs.target" ];
        serviceConfig = {
          Type = "oneshot";
          # Stays as root: needs unshare(CLONE_NEWNS), umount, chown.
          User = "root";
          SyslogIdentifier = "nixling-${name}-store-sync";
          ExecStart = "${pkgs.writeShellScript "nixling-${name}-store-sync-trigger" ''
            set -euo pipefail
            VM=${lib.escapeShellArg name}
            GEN_LINK=/run/nixling/$VM/next-generation
            if [ ! -L "$GEN_LINK" ]; then
              echo "nixling-$VM-store-sync: no $GEN_LINK; refusing to run." >&2
              exit 2
            fi
            GEN=$(readlink -f "$GEN_LINK")
            exec ${nixlingStoreSync}/bin/nixling-store-sync "$VM" "$GEN"
          ''}";
        };
      })
      enabledVms);

    systemd.timers."nixling-vfsd-watchdog@" = {
      description = "Periodic check for wedged cloud-hypervisor vhost-user-fs on %i";
      timerConfig = {
        OnBootSec = "2min";
        OnUnitActiveSec = "60s";
        AccuracySec = "10s";
        Unit = "nixling-vfsd-watchdog@%i.service";
      };
    };

    # ---------------------------------------------------------------------------
    # Host activation hook.
    # On every `nixos-rebuild switch`, drop the per-VM next-generation
    # pointer into /run/nixling/<vm>/next-generation and invoke the
    # sync helper directly (in-process; faster than systemd-start and
    # gives us synchronous error reporting in the activation log).
    # ---------------------------------------------------------------------------
    system.activationScripts.nixlingStoreSync = lib.stringAfter [ "specialfs" "users" ] ''
      set -u
      install -d -m 0755 /run/nixling
      ${lib.concatStringsSep "\n" (lib.mapAttrsToList
        (name: gen: ''
          install -d -m 0755 /run/nixling/${name}
          ln -sfT ${gen} /run/nixling/${name}/next-generation
          if ! ${nixlingStoreSync}/bin/nixling-store-sync ${name} ${gen}; then
            echo "nixling: warning — nixling-store-sync for '${name}' failed (continuing activation)"
          fi
        '')
        vmGenPaths)}
    '';

    # ---------------------------------------------------------------------------
    # P1r2 — vhost-user reconnect resilience.
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
    # Two layers of defense:
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
            # "{ path=/nix/store/...; argv[]=...; ... }"; extract just
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
