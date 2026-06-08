# crosvm seccomp policy override used by the GPU sidecar. Originally
# an inline let-block; lifted into the framework via phase-1c so
# consumers can depend on a single store path. Paired with
# `pkgs/crosvm-patched/` which builds the crosvm binary itself.
{ pkgs }:

let
  # The seccomp .policy files that ship with the crosvm rev pinned
  # in nixpkgs (Feb 2026, rev 4c80bf3) predate upstream's policy
  # update for glibc 2.41+ + Linux 6.13+: the device proxies'
  # `madvise` allowlist doesn't include the new `MADV_GUARD_INSTALL`
  # / `MADV_GUARD_REMOVE` constants that glibc's pthread stack
  # setup now uses. The xhci controller proxy crashes with SIGSYS
  # on syscall=28 the moment its EventLoop spawns its first thread.
  #
  # Rather than carry hand-rolled patches, fetch the seccomp dir
  # at a known-good newer crosvm rev (the commit that added
  # MADV_GUARD_*) and use those policies. Policy files are pure
  # text and forward-compatible with the slightly older crosvm
  # binary — newer versions only add allowed syscalls/args.
  #
  # Pinned rev: google/crosvm@299c1e7 ("seccomp: Add
  # MADV_GUARD_{INSTALL,REMOVE}", 2026-03-27).
  crosvmSeccompSrc = pkgs.fetchFromGitHub {
    owner = "google";
    repo = "crosvm";
    rev = "299c1e7c3d5a1b98106212c20f58b9fdb7b1b1ea";
    hash = "sha256-JQGrxY79OMAXOgVKI9rMbBZSppHtssDxrpsDdGmrzGU=";
    sparseCheckout = [ "jail/seccomp/x86_64" ];
  };

  crosvmSeccomp = pkgs.runCommand "crosvm-seccomp-policies-x86_64"
    {
      nativeBuildInputs = [ pkgs.minijail-tools ];
      # Phase 4 multi-arch: pinned crosvm sparseCheckout is the
      # `jail/seccomp/x86_64/` directory, and the compiled .bpf files
      # are produced via minijail-tools' x86_64 syscall table. There
      # is no aarch64 counterpart to load here (crosvm's aarch64 jail
      # policies live in a sibling dir that we don't fetch). Marking
      # the derivation x86_64-only signals intent and keeps any
      # downstream `nix flake check --all-systems` honest about which
      # arch this package can run on.
      meta.platforms = [ "x86_64-linux" ];
    } ''
    mkdir -p $out/share/policy/crosvm
    cp ${crosvmSeccompSrc}/jail/seccomp/x86_64/* $out/share/policy/crosvm/
    chmod -R u+w $out/share/policy/crosvm

    # Rewrite the upstream `@include /usr/share/policy/crosvm/...`
    # prefixes in case the .policy fallback path is ever taken.
    sed -i \
      "s|/usr/share/policy/crosvm/|$out/share/policy/crosvm/|g" \
      $out/share/policy/crosvm/*.policy

    # nixpkgs glibc 2.41+ uses `statx` (syscall 332) in place of
    # `stat`/`fstat` for several internal lookups (CWD resolution,
    # NSS module probing, dlopen path stat, getrandom poll). The
    # pcivirtio-net and pcivirtio-rng device proxies hit it during
    # init and SIGSYS out, because their policies (net.policy /
    # rng_device.policy) include common_device.policy but neither
    # they nor common allow statx. statx is a metadata-read
    # syscall, no capability grant — safe to allow for any device
    # proxy that includes common_device.policy.
    #
    # Several other policies (9p_device, block, fs_device, etc.)
    # already define `statx: 1` unconditionally on their own. To
    # avoid the Python compiler's "syscall already had an
    # unconditional action applied" rejection on those, strip
    # per-device duplicates first, then add the single canonical
    # allow to common_device.policy.
    find $out/share/policy/crosvm -name '*.policy' \
      ! -name 'common_device.policy' \
      -exec sed -i '/^statx:/d' {} +
    cat >> $out/share/policy/crosvm/common_device.policy <<'EOF'

# nixpkgs glibc 2.41+ compat — see modules/nixling/graphics.nix
statx: 1
EOF

    # Pre-compile every .policy to a .bpf using the Python compiler,
    # which (unlike the C one in libminijail) correctly merges
    # duplicate syscall definitions across @include chains.
    cd $out/share/policy/crosvm
    for p in *.policy; do
      compile_seccomp_policy "$p" "''${p%.policy}.bpf"
    done
  '';
in
crosvmSeccomp
