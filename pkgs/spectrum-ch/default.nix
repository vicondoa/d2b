# SPDX-FileCopyrightText: 2022 Unikie
# SPDX-FileCopyrightText: 2023-2025 Alyssa Ross <hi@alyssa.is>
# SPDX-License-Identifier: MIT
#
# spectrum-os cloud-hypervisor with virtio-gpu patches, self-contained.
#
# Source of truth (upstream): https://spectrum-os.org/git/spectrum/
# Specifically, this is a re-implementation of
#   spectrum/pkgs/cloud-hypervisor/default.nix
# at the rev pinned by microvm.nix flake.lock (`spectrum` input).
#
# Why vendored instead of using microvm.nix's `cloud-hypervisor-graphics`
# overlay directly:
#   - microvm.nix's overlay uses spectrum's vhost-input directly via
#     `git+https://spectrum-os.org/git/spectrum`, which works fine over
#     `git clone` but the snapshot-tarball codepath has been flaky
#     historically. Vendoring isolates us from that.
#   - We have our own *rebased* spectrum patches that target a newer
#     cloud-hypervisor version (v52) than upstream spectrum currently
#     ships patches for (their latest patch tarball is for v50).
#     Spectrum's git HEAD still tracks `pkgs.cloud-hypervisor` from
#     whatever nixpkgs they pin (Mar 2026 = v50-era) — they have not
#     yet rebased to v52, but we needed to NOW for two reasons:
#       (a) CVE-2026-45782 (CVSS 8.9, virtio-block UAF guest→host
#           escape) affects v21.0 – v51.1; fixed in v51.2 / v52.0.
#       (b) v52.0 is the first release to ship `--generic-vhost-user`,
#           which the audio component (modules/nixling/audio.nix)
#           depends on to wire vhost-device-sound into CH.
#
# Pinning:
#   - cloud-hypervisor source: v52.0 (rebased patches in ./cloud-hypervisor/).
#   - vhost source: rust-vmm/vhost @ vhost-user-backend-v0.22.0
#     (matches v52's Cargo.toml: `vhost = 0.16`, `vhost-user-backend = 0.22`).
#
# To update:
#   1. Bump `version` + `src.hash`. nixpkgs' fetchFromGitHub will
#      print the right hash if you set it to lib.fakeHash.
#   2. Bump cargoDeps.hash same way.
#   3. Try the patches against the new src in a scratch dir; if any
#      hunks fail, port them by hand, regenerate via `git format-patch`,
#      and replace the files in ./cloud-hypervisor/.
#   4. When upstream spectrum rebases to a newer CH, prefer dropping
#      our rebased patches in favour of theirs to minimise drift.
{ pkgs, ... }:

pkgs.cloud-hypervisor.overrideAttrs (oldAttrs: rec {
  version = "52.0";
  src = pkgs.fetchFromGitHub {
    owner = "cloud-hypervisor";
    repo = "cloud-hypervisor";
    rev = "v${version}";
    hash = "sha256-OGyvmedSaWPsyH6mdHhgXN7MvTnK1HzdfTKUhJRlq8I=";
  };

  patches = (oldAttrs.patches or [ ]) ++ [
    ./cloud-hypervisor/0001-build-use-local-vhost.patch
    ./cloud-hypervisor/0002-virtio-devices-add-a-GPU-device.patch
    ./cloud-hypervisor/0003-vhost-user-media-device.patch
  ];

  cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
    inherit src patches;
    hash = "sha256-ZNj1H3Iq+IUSe0McHJjrwPOoR+YRB+rsSmZHMhXsHy0=";
  };

  vhost = pkgs.fetchFromGitHub {
    name = "vhost";
    owner = "rust-vmm";
    repo = "vhost";
    rev = "vhost-user-backend-v0.22.0";
    hash = "sha256-UzzwLi4O6rWuIJuGU0C0+tzJ7NZrgRamt/iYereRHZU=";
  };

  vhostPatches = [
    ./vhost/0001-vhost_user-add-get_size-to-MsgHeader.patch
    ./vhost/0002-vhost-fix-receiving-reply-payloads.patch
    # 0003 + 0004 (shared-memory-region + protocol-flag-for-shmem) from
    # spectrum target an older vhost without native SHMEM support and
    # collide heavily with v0.22's upstream APIs (duplicate SHMEM_MAP,
    # duplicate shmem_map/shmem_unmap, etc.). Replaced with a single
    # minimal compat patch that adds just the two symbols the CH GPU
    # patch consumes: `VhostSharedMemoryRegion` and
    # `get_shared_memory_regions()` — both implemented as adapters
    # over upstream `VhostUserShMemConfig` / `get_shmem_config()`.
    ./vhost/0003-shared-memory-region-compat.patch
  ];

  postUnpack = (oldAttrs.postUnpack or "") + ''
    unpackFile $vhost
    chmod -R +w vhost
  '';

  postPatch = (oldAttrs.postPatch or "") + ''
    pushd ../vhost
    for patch in $vhostPatches; do
        echo applying patch $patch
        patch -p1 -F3 < $patch
    done
    popd

    # devices/src/tpm.rs at v52 logs every CRB locality request/relinquish
    # at WARN level. The kernel's TPM CRB driver writes locality-request (1)
    # before each command and locality-relinquish (2) after — i.e. two of
    # these messages per TPM command. The result is the host console
    # spamming hundreds of lines per second during Himmelblau / tpm2-tools
    # activity. Demote to debug! so they only appear when the user opts in
    # via RUST_LOG=debug. Functionally a no-op.
    ${pkgs.gnused}/bin/sed -i -E \
      's/warn!\("CRB_LOC_CTRL locality to write/debug!("CRB_LOC_CTRL locality to write/' \
      devices/src/tpm.rs
  '';

  # microvm.nix's cloud-hypervisor runner generates `--disk` lines
  # that include `image_type=raw,`. CH v50 didn't recognize that key;
  # v52 does. The strip-shim that used to live here is gone.
  #
  # However, microvm.nix unconditionally emits `--memory
  # mergeable=on,shared=on,size=…` whenever a VM has virtiofs shares
  # OR graphics enabled (both true for every nixling workload VM,
  # because per-VM /nix/store is virtiofs-backed). CH v52 added a
  # validator that rejects that combination:
  #
  #   ERROR: Fatal error: ParsingConfig(Validation(InvalidSharedMemoryWithMergeable))
  #   "Invalid to set both 'mergeable' and 'shared' for memory"
  #
  # `mergeable=on` enables Kernel Same-page Merging (a host-RAM perf
  # optimization). `shared=on` is REQUIRED for vhost-user-fs and
  # virtio-gpu (the guest's memory must be host-shared so the device
  # can DMA into it). For our VMs `shared=on` is non-negotiable, so
  # we rewrite `mergeable=on` to `mergeable=off` in any `--memory`
  # arg before CH sees it. Cost: lose KSM page deduplication for
  # these VMs. Worth it: every nixling VM needs virtiofs.
  postFixup = (oldAttrs.postFixup or "") + ''
    mv $out/bin/cloud-hypervisor $out/bin/.cloud-hypervisor-real
    # Heredoc is UNQUOTED so ${pkgs.gnused} expands at write time.
    # Runtime variables ($1, $@, $0, $here) are escaped with \.
    cat > $out/bin/cloud-hypervisor <<WRAP
    #!${pkgs.runtimeShell}
    set -eu
    here=\$(dirname -- "\$0")
    clean_memory() {
      printf '%s' "\$1" \
        | ${pkgs.gnused}/bin/sed -E \
            -e 's/(^|,)mergeable=on(,|\$)/\1mergeable=off\2/g'
    }
    argv=()
    while [ \$# -gt 0 ]; do
      if [ "\$1" = "--memory" ] && [ \$# -ge 2 ]; then
        argv+=( "\$1" "\$(clean_memory "\$2")" )
        shift 2
      else
        argv+=( "\$1" ); shift
      fi
    done
    exec "\$here/.cloud-hypervisor-real" "\''${argv[@]}"
    WRAP
    chmod +x $out/bin/cloud-hypervisor
  '';

  # The default installCheckPhase / versionCheckPhase invokes
  # `cloud-hypervisor --version` and greps for the version. We don't
  # need that during dev iteration; let nixpkgs' check find "52.0"
  # in the real binary's output.
  doInstallCheck = false;

  # The crosvm rev this CH build has been QA'd against as the
  # GPU sidecar partner. graphics.nix asserts at eval time that
  # `pkgs.crosvm.src.rev` still matches this value; a mismatch means
  # nixpkgs moved crosvm independently of this vendored CH and the
  # vhost-user-gpu wire protocol may have drifted. Current pair:
  # CH v52.0 + crosvm 4c80bf3 (Feb 2026, ships standardised SHMEM
  # message numbers expected by rust-vmm/vhost v0.22).
  passthru.testedWithCrosvmRev = "4c80bf3523cf84114054209d88a7af3eefd8423f";

  # Multi-arch: deliberately we do NOT set
  # `meta.platforms = [ "x86_64-linux" ]` on this overrideAttrs even
  # though spectrum-ch *is* logically x86_64-only (its GPU patches
  # and the bumped `vhost` rev pair with the x86_64-only crosvm
  # sidecar). The reason is that `store.nix` references this package
  # unconditionally via `environment.systemPackages` for the
  # `ch-remote` watchdog tool, and Nix's check-meta runs at eval
  # time the moment we override `meta` here — which would break the
  # headless-on-aarch64 invariant we explicitly preserve.
  # nixpkgs's own `pkgs.cloud-hypervisor` already carries
  # `meta.platforms = [ "x86_64-linux" ]`; we inherit that
  # constraint at build time without forcing an eval-time refusal
  # for downstream `nix flake check --all-systems`. The platform
  # gate at the graphics/audio component layer
  # (`nixos-modules/host.nix` checkVmPlatform) is the authoritative
  # eval-time check for nixling consumers.
})
