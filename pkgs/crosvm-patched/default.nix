# crosvm with patched seccomp policies for the GPU sidecar used by
# nixling's graphics component (see
# `nixos-modules/components/graphics.nix`). Originally an inline
# let-block; lifted into the framework via phase-1c so consumers can
# depend on a single store path. See `pkgs/crosvm-seccomp/` for the
# accompanying policy override.
#
# NOTE on minijail-patched: an earlier revision of the let-block also
# defined `minijailPatched` (a `pkgs.minijail.overrideAttrs` with
# ALLOW_DUPLICATE_SYSCALLS=yes). That override was eliminated when the
# .policy → .bpf precompile step was added to crosvm-seccomp (the
# Python compile_seccomp_policy tool correctly merges duplicate
# syscall definitions, so the C libminijail parser is never invoked).
# See the comment block in `nixos-modules/components/graphics.nix`
# for the full story. Hence: no `pkgs/minijail-patched/` is created.
{ pkgs }:

let
  crosvmSeccomp = import ../crosvm-seccomp { inherit pkgs; };

  # security-r8-audio-11: port of talex5/crosvm@993b8e756 "Don't open
  # a graphical console window" to the vhost-user-gpu backend. See
  # the patch file for the full rationale. Without this, every
  # graphics VM start creates a chromeless, transparent, undecorated
  # window titled "crosvm" on the host compositor.
  crosvmNoGraphicalConsole = pkgs.crosvm.overrideAttrs (old: {
    patches = (old.patches or [ ]) ++ [
      ../patches/crosvm-no-graphical-console.patch
    ];
  });

  # The GPU sidecar is `crosvm device gpu`, spawned by microvm.nix's
  # cloud-hypervisor runner over vhost-user-gpu. Use the crosvm
  # nixpkgs ships (Feb 2026, rev 4c80bf3) directly — that rev speaks
  # the standardised vhost-user shmem message numbers
  # (`GET_SHMEM_CONFIG = 44`, `SHMEM_MAP = 9`, `SHMEM_UNMAP = 10`)
  # which match rust-vmm/vhost @ vhost-user-backend-v0.22.0 (the
  # vhost crate we use in pkgs/spectrum-ch/ for
  # cloud-hypervisor v52).
  #
  # Historical note: this used to be pinned to crosvm 18bc84d
  # (Oct 2024), which used the OLD non-standard numbers
  # (`GET_SHARED_MEMORY_REGIONS = 1004`, `SHMEM_MAP = 1000`,
  # `SHMEM_UNMAP = 1001`) — those are what spectrum's CH 50 patch
  # series sent. The CH v50 -> v52 bump took us to the
  # standardised number set, and we now need the matching crosvm
  # (any rev >= the Dec 2025 commit 729f98c "Update GET_SHMEM_CONFIG
  # messages"). Nixpkgs's pin sits comfortably after that.
  #
  # We no longer override minijail with ALLOW_DUPLICATE_SYSCALLS=yes:
  # all .policy files are pre-compiled to .bpf below using the
  # Python `compile_seccomp_policy` tool (which correctly merges
  # duplicate syscall definitions across @include chains). The C
  # libminijail parser is only invoked when crosvm finds a .policy
  # file without a matching .bpf, which never happens here because
  # crosvmSeccomp pre-compiles every policy. The previous statx
  # collision (the only known duplicate-syscall case) is resolved
  # by stripping per-device `statx:` lines and re-adding a single
  # canonical allow in common_device.policy (see crosvmSeccomp).
  #
  # nixos-1 (P5 round-1): symlinkJoin pkgs.crosvm with crosvmSeccomp
  # so the compiled .bpf files live alongside the crosvm binary
  # under the same store path. crosvm's jail loader looks for
  # ${out}/share/policy/crosvm/<device>.bpf relative to its own
  # binary; without the join, only the upstream .policy files
  # shipped by nixpkgs are reachable and the .bpf precompile work
  # is silently bypassed (so duplicate-syscall failures recur the
  # first time the C parser sees a policy with @includes).
  #
  # rust-r2-1 (P5 round-2): KNOWN LIMITATION — seccomp policies not
  # loaded at runtime by `crosvm device gpu`.
  #
  # The symlinkJoin above adds the compiled .bpf files to the package
  # closure and places them at the path crosvm's jail loader expects
  # (${out}/share/policy/crosvm/). However, `crosvm device gpu` (the
  # subcommand microvm.nix invokes as the vhost-user-gpu sidecar) has
  # NO --seccomp-policy-dir flag in this crosvm rev (Feb 2026,
  # 4c80bf3). Verified: `crosvm device gpu --help` exposes only
  # --socket-path, --fd, --wayland-sock, --resource-bridge,
  # --x-display, and --params — no seccomp knob.
  #
  # Loading seccomp policies at runtime for the gpu device subcommand
  # requires a crosvm-side change. We retain the .bpf files in the
  # closure as defence-in-depth: when a future crosvm rev exposes
  # --seccomp-policy-dir on `device gpu`, the policies will already
  # be present and we only need to wire the flag here.
  #
  # TODO(rust-r2-1): when crosvm device gpu gains --seccomp-policy-dir,
  # update the graphics.crosvmPackage / shim invocation in this file
  # to pass --seccomp-policy-dir=${crosvmPatched}/share/policy/crosvm
  # and update test_crosvm_gpu_seccomp_loaded to verify Seccomp:2.
  crosvmPatched = pkgs.symlinkJoin {
    name = "crosvm-with-seccomp";
    paths = [ crosvmNoGraphicalConsole crosvmSeccomp ];
    # Phase 4 multi-arch: the join inherits its components' platform
    # support. crosvmSeccomp is x86_64-only (see its meta block) and
    # crosvmNoGraphicalConsole patches `pkgs.crosvm` which nixpkgs
    # itself restricts to x86_64-linux. Make the constraint explicit
    # so any downstream `nix flake check --all-systems` sees it.
    meta.platforms = [ "x86_64-linux" ];
  };
in
crosvmPatched
