# `pkgs/spectrum-ch/` — update process

This module vendors a self-contained build of cloud-hypervisor with
[spectrum-os]'s virtio-gpu patches plus four `rust-vmm/vhost`
backports, then wraps the resulting binary in a shell shim that
strips a CH-51+-only `--disk` option that microvm.nix generates.

[spectrum-os]: https://spectrum-os.org/software/cloud-hypervisor/

## What's pinned and why

| Pin | Where | Why |
| --- | --- | --- |
| cloud-hypervisor **v50.0** | `version` / `src.hash` | spectrum-os only publishes patches for v50; CH 51+ rejects the patch hunks. |
| rust-vmm/vhost **vhost-user-backend-v0.20.0** | `vhost` `fetchFromGitHub` | matches the API the spectrum vhost-user patches expect. |
| 2 CH patches | `cloud-hypervisor/*.patch` | virtio-gpu device + local-vhost wiring. |
| 4 vhost patches | `vhost/*.patch` | shmem/get_size/protocol-flag backports needed by virtio-gpu. |
| `cargoDeps` hash | `pkgs.rustPlatform.fetchCargoVendor` | content-addressed lock for the patched-source Cargo deps; bump in lockstep with `version` / patches. |
| 1 sed (CRB log demotion) | `postPatch` | demotes per-TPM-command `CRB_LOC_CTRL` warnings to debug! so the host console doesn't get spammed during Himmelblau activity. |

## Why we vendor (don't use microvm.nix's overlay)

microvm.nix has a `cloud-hypervisor-graphics` overlay that does the
spectrum-os patching. Two problems:

1. **spectrum-os.org's git server consistently truncates snapshot
   tarballs** below ~100KB (HTTP/2 RST / index-pack truncation) and
   full git clones fail with `"fetch-pack: invalid index-pack
   output"`. Result: the overlay can't materialise its source.
2. The patches are ~93KB across 6 small files and the cgit
   `/plain/` endpoint returns them fine individually. Vendoring is
   small, reproducible, and removes runtime dependency on
   spectrum-os.org being healthy.

## Bumping to a newer spectrum-os patch release

1. Check <https://spectrum-os.org/software/cloud-hypervisor/> for a
   newer patch tarball ("`cloud-hypervisor-NN.0-spectrum-vN.tar.xz`").
2. **Don't try the snapshot tarball.** Pull each `.patch` file from
   the cgit `/plain/` path one-by-one:
   ```
   curl -L -o pkgs/spectrum-ch/cloud-hypervisor/0001-...patch \
     'https://spectrum-os.org/git/spectrum/plain/pkgs/cloud-hypervisor/0001-...patch?id=<rev>'
   ```
   Make sure each file is non-empty before committing — failed
   fetches return short HTML error bodies, not patches.
3. Bump `version` to whatever CH release the new patches target;
   bump `src.hash` (use `nix-prefetch-url --unpack
   https://github.com/cloud-hypervisor/cloud-hypervisor/archive/vNN.0.tar.gz`).
4. **Bump `cargoDeps.hash` to `""`** then run `nixos-rebuild build`;
   nix will print the correct hash. Paste it in.
5. **Check the postFixup shim is still needed**: if the new
   spectrum-patched CH version >= 51, the `image_type=` strip can
   probably go away (microvm.nix accepts that option natively in
   51+).
6. Test in this order:
   - Build succeeds: `nix build .#nixosConfigurations.<host>.config.system.build.toplevel`.
   - `nixling up <graphics-vm>` brings a graphics-enabled VM up cleanly.
   - `ch-remote info` against the running VM doesn't error.
   - Virtio-gpu DOES still render the VM display (this is what the
     patches are for — without them you get a black or missing
     window).
   - TPM still works inside the VM (`tpm2_getrandom -T device:/dev/tpmrm0 4`).

## Bumping `rust-vmm/vhost`

Strongly **don't** unless a CVE forces it. The four backported
patches assume API shapes from v0.20.0; newer vhost has reshuffled
the message-handler traits and the patches will need rewriting.

If you must:
1. Check whether the four patches' contents are already upstream in
   the new version. They probably are by now — try without our
   patches first and see if the build / runtime works.
2. If a fix landed upstream, delete the corresponding file from
   `vhostPatches`.

## Sharp edges

- **`postUnpack` unpacks `$vhost` as a sibling** to the
  cloud-hypervisor source tree (not into it). The patch step
  `pushd ../vhost` relies on that layout.
- **`doInstallCheck = false`** because the postFixup shim renames
  the real binary and the version check then greps the wrapper —
  it forwards correctly at runtime but the `--version` self-check
  doesn't recognise it.
- **The CRB sed is a log-only change** — purely cosmetic. If a
  future CH refactors `devices/src/tpm.rs` it stops applying
  silently (sed exits 0 on no-match). Add a `grep` verifier if you
  care.
- **`--disk image_type=raw,` strip shim** — microvm.nix unconditionally
  emits this on cloud-hypervisor since some recent rev. CH 50.0 does
  not know the option and refuses to start. The shim greps it out
  with sed at boot. Equivalent in spirit to the crosvm GPU sidecar
  wrapper used by `components/graphics.nix`. The shim's heredoc is
  UNQUOTED so `${pkgs.gnused}` expands at build time, but runtime
  `$1` / `$@` / `$here` are escaped with `\`.

## When to delete this module

If microvm.nix's upstream `cloud-hypervisor-graphics` overlay starts
working reliably (i.e. spectrum-os.org's git server is fixed or
microvm.nix mirrors the patches), this whole module collapses to:

```nix
microvm.hypervisor = "cloud-hypervisor";
microvm.graphics.enable = true;
```

…and the only thing left to keep would be the CRB-log demotion sed
(which you could keep as an overlay one-liner on
`pkgs.cloud-hypervisor`).

## L1.7 — cargoVendor hash verification

When bumping CH (`pkgs/spectrum-ch/default.nix`) or the crosvm pin used for the GPU sidecar,
manually verify the `cargoDeps` / `cargoVendor` hash by running `nix-build` and comparing to
upstream `Cargo.lock` SHA-256s. A mis-hashed vendor tree silently pulls in whichever packages
were in the store at hash-collision time.

### Verification snippet

```bash
cd $(mktemp -d) && nix flake clone github:cloud-hypervisor/cloud-hypervisor && cargo vendor --locked > /tmp/vendor.lock && sha256sum /tmp/vendor.lock
```

Compare the printed SHA-256 against the `cargoDeps.hash` in `default.nix`. If they differ,
the vendored tree is stale or wrong — regenerate with:

```bash
nix build .#nixosConfigurations.<host>.config.microvm.vms.<graphics-vm>.config.microvm.hypervisor.package --print-out-paths
```

Also update `passthru.testedWithCrosvmRev` in `default.nix` whenever you change which crosvm
rev the GPU sidecar uses (see `nixos-modules/components/graphics.nix`). The build-time assertion in
`graphics.nix` will fail if the CH package's `testedWithCrosvmRev` doesn't match
`pkgs.crosvm.src.rev`.

### Bump cadence

- CH + crosvm: quarterly, or immediately on a published CVE.
- Check https://github.com/cloud-hypervisor/cloud-hypervisor/releases and
  https://chromium.googlesource.com/chromiumos/platform/crosvm/+log for security advisories.
- After each bump, run `bash tests/static.sh` from the framework checkout, then the relevant
  Layer-2 integration tests (`tests/nixling-store.sh --quick`, `tests/audio.sh --quick`)
  on a live host before committing.
