{
  description = "Venus Vulkan Video lab: H.264 decode forwarding for unmodified Firefox";

  # Self-contained on purpose. This flake does NOT reference the root d2b
  # flake, and the root flake does not reference it. It pins its own nixpkgs
  # because the prototype needs Firefox 153, which is newer than d2b's pin.
  # See AGENTS.md rule 4 for why nothing mutable may live inside this tree.
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/38a4887411571457d700c51c64a6e49ead2ed5ab";

    # The three forks. Marked `flake = false`: these are plain source trees.
    # Each is pinned to its `vulkan-video` branch and carries a `base/<rev>`
    # tag recording the exact upstream commit it was seeded from -- the W1
    # append-only ABI gate and the W4 compatibility cross-product diff against
    # those tags, so they must never move.
    venus-protocol-src = {
      url = "github:vicondoa/venus-protocol-vulkan-video/vulkan-video";
      flake = false;
    };

    # The base revision each fork was seeded from. The ABI gate regenerates
    # this to prove that every changed serialization function only GAINED
    # lines, which is what makes keeping VN_WIRE_FORMAT_VERSION at 1 safe.
    venus-protocol-base = {
      url = "github:vicondoa/venus-protocol-vulkan-video/70991d4c7e4e5a7bfa2fbb8a6e77e4eac350145d";
      flake = false;
    };
    # The upstream revision the virglrenderer fork was seeded from, lock-pinned
    # so pins-check can verify PINS.md against it rather than trusting prose.
    # There is deliberately no mesa-base input: the mesa fork was rebased onto
    # 26.1, so its base/ tag is no longer an ancestor of the fork branch and
    # cannot be fetched from the remote. PINS.md marks that row accordingly.
    virglrenderer-base = {
      url = "github:vicondoa/virglrenderer-venus-vulkan-video/9ae1fb1cca8ec884f5e44d6ae09425288aeae4cd";
      flake = false;
    };

    # Temporarily on virgl-video-enable rather than vulkan-video. That branch
    # is 335be0b7 (the revision v3 pins) plus the virgl-video enablement, and
    # deliberately excludes add87c05: the blit change on vulkan-video is
    # unproven, and compounding it with this would make a measurement of
    # either one unattributable.
    virglrenderer-src = {
      url = "github:vicondoa/virglrenderer-venus-vulkan-video/virgl-video-enable";
      flake = false;
    };
    mesa-src = {
      url = "github:vicondoa/mesa-venus-vulkan-video/vulkan-video";
      flake = false;
    };
  };

  outputs =
    { self, nixpkgs, venus-protocol-src, venus-protocol-base, virglrenderer-src
    , virglrenderer-base, mesa-src }:
    let
      system = "x86_64-linux";

      # HOST and GUEST package sets are deliberately separate objects.
      #
      # Two different Mesa builds exist in this lab and confusing them is a
      # documented failure mode (AGENTS.md rule 5):
      #   * guest -> patched lab Mesa (the Venus ICD we are adding video to)
      #   * host  -> stock Mesa; only virglrenderer is patched
      #
      # Keeping them as distinct sets means no single global overlay can
      # silently cross the boundary.
      hostPkgs = import nixpkgs { inherit system; };
      guestPkgs = import nixpkgs { inherit system; };

      # ---------------------------------------------------------------------
      # Host side: virglrenderer built from the fork.
      # ---------------------------------------------------------------------
      # venus-protocol headers are VENDORED in-tree at
      # src/venus/venus-protocol/ (46 generated `vn_protocol_renderer_*.h`
      # files plus a vk_video/ directory that already contains the StdVideo
      # H.264 headers). There is therefore no separate venus-protocol
      # dependency to wire in here: W1's generated output is copied into the
      # fork, which this derivation then builds.
      labVirglrenderer = hostPkgs.virglrenderer.overrideAttrs (old: {
        pname = "virglrenderer-venus-vulkan-video";
        version = "lab-${virglrenderer-src.shortRev or "dirty"}";
        src = virglrenderer-src;
        # Upstream nixpkgs carries a patch for the pinned 1.3.0 tarball that
        # does not apply to the fork's newer tree.
        patches = [ ];
        passthru = (old.passthru or { }) // {
          labVenusProtocolSrc = venus-protocol-src;
        };
      });

      # crosvm relinked against the LAB virglrenderer.
      #
      # This MUST be a real `override`, not a symlinkJoin: crosvm resolves
      # libvirglrenderer through its RPATH, so a join would keep loading
      # nixpkgs' stock build while appearing to work. `apps.prove-crosvm-binding`
      # exists specifically to catch that.
      #
      # Patch policy differs from d2b's production build on purpose:
      #   KEEP crosvm-gpu-device-node.patch -- it ADDS the --gpu-device-node
      #        flag, which is not upstream. Without it virglrenderer never
      #        gets a DRM fd for the host GPU.
      #   DROP crosvm-no-graphical-console.patch -- production suppresses the
      #        console window; a lab WANTS a visible window to render into.
      labCrosvm = (hostPkgs.crosvm.override {
        virglrenderer = labVirglrenderer;
      }).overrideAttrs (old: {
        pname = "crosvm-venus-vulkan-video";
        patches = (old.patches or [ ]) ++ [ ./pkgs/patches/crosvm-gpu-device-node.patch ];
      });

      # Cloud Hypervisor with the spectrum-os virtio-gpu patches. Stock CH has
      # no --gpu device, so the vendored patch set is required: the whole point
      # is attaching the crosvm GPU sidecar over vhost-user-gpu.
      #
      # Checks are disabled because upstream CH's block-layer unit tests
      # exercise io_uring, which the Nix build sandbox blocks; 20 of them fail
      # for that reason alone. They test qcow/vhd behaviour this lab does not
      # use and would not be validating anyway.
      labCloudHypervisor =
        (import ./pkgs/spectrum-ch { pkgs = hostPkgs; }).overrideAttrs (_: {
          doCheck = false;
        });

      # ---------------------------------------------------------------------
      # Guest side: Mesa built from the fork (the Venus ICD).
      # ---------------------------------------------------------------------
      # The fork's vulkan-video branch tracks upstream's 26.1 branch rather
      # than main, so the source matches the version nixpkgs' derivation is
      # written against (26.1.5). Mesa main has drifted enough that nixpkgs'
      # opencl.patch no longer applies, and that patch is what introduces the
      # `clang-libdir` meson option the derivation passes.
      #
      # Do NOT clear `patches` here for the same reason: dropping nixpkgs'
      # patch list removes opencl.patch and the build fails with
      # `ERROR: Unknown option: "clang-libdir"`.
      #
      # 26.1 is also the right target on the merits: the passthrough extension
      # table is identical between main and 26.1, MR !35842's format scrubbing
      # is present in 26.1.x, and d2b's deployed guests already run 26.1.x.
      labMesa = guestPkgs.mesa.overrideAttrs (old: {
        pname = "mesa-venus-vulkan-video";
        version = "lab-${mesa-src.shortRev or "dirty"}";
        src = mesa-src;
      });

      # ---------------------------------------------------------------------
      # Guest side: STOCK Firefox. No source patches, by definition.
      # ---------------------------------------------------------------------
      # MOZ_ENABLE_VULKAN_VIDEO needs no build flag: toolkit/moz.configure sets
      # it unconditionally for GTK builds, and nixpkgs builds Firefox with
      # cairo-gtk3-wayland. The only customization here is preferences, which
      # is explicitly allowed by the prototype's success criteria.
      #
      # direct-export is OFF: Firefox only ever requests the modifier-tiled
      # export shape, and that exact query is refused by the host NVIDIA driver
      # itself. See the pref comment below. Turning it off selects a non-export
      # route; which of the two non-export routes Firefox then takes is decided
      # separately, by whether zero-copy is configured.
      # WebM/VP9 is disabled so YouTube serves H.264 -- required permanently,
      # since no NVIDIA driver implements VK_KHR_video_decode_vp9 and Turing
      # has no AV1 engine.
      labFirefox = guestPkgs.wrapFirefox guestPkgs.firefox-unwrapped {
        # gfx.blacklist.hardwarevideodecoding is deliberately NOT set here any
        # more, and that is the point of the virgl-video work.
        #
        # It used to be. gfxPlatformGtk's InitPlatformHardwareVideoConfig
        # returns early unless HARDWARE_VIDEO_DECODING is enabled, and only
        # past that early return is HW_DECODED_VIDEO_ZERO_COPY configured.
        # With zero-copy unset, VideoFramePool::ShouldCopySurface returns true
        # unconditionally and every frame takes the broken copy path. So the
        # pref was set to 1 (FEATURE_STATUS_OK) to skip the VA-API probe that
        # gates that feature.
        #
        # That was a diagnostic rather than a fix, and it was written down as
        # one: it asserted a capability the guest did not have. The probe could
        # not pass honestly, because the guest's virtio_gpu VA driver loaded
        # and initialised and then advertised no H.264 profiles at all.
        #
        # It advertises them now. crosvm never passes VIRGL_RENDERER_USE_VIDEO,
        # so virglrenderer never called virgl_video_init(), so va_dpy stayed
        # NULL and the virgl2 capset reached the guest with num_video_caps = 0.
        # With video initialised the guest reports H264 ConstrainedBaseline,
        # Main and High, so Firefox reaches its own conclusion from what the
        # driver reports instead of having the probe bypassed.
        #
        # That advertisement has NOT been shown to be hardware backed. A guest
        # VA-API decode was later measured against the same decode on the host:
        # the host reached 94-98% NVDEC, the guest reached 0% on every sample
        # while running 5.7x faster than the real decoder. So this removes a
        # bypass rather than establishing a capability. See SOLUTION.md 6a.
        #
        # It does not affect what decodes. InitHWDecoderIfAllowed tries
        # InitVulkanDecoder() before InitVAAPIDecoder(), so Vulkan Video decodes
        # every frame and VA-API is never used for one.
        extraPolicies = {
          DisableTelemetry = true;
          DisableFirefoxStudies = true;
          Preferences = {
            # NOT locked, deliberately.
            #
            # Locking this reads like rigour -- it guarantees the decoder is on
            # and cannot be perturbed -- but it makes the positive claim
            # unfalsifiable. The evidence contract requires a negative control
            # for every positive claim, and the cheapest one by far is to turn
            # the decoder off in the live session and show the renderer's
            # decode counters stop moving. A locked pref silently refuses that
            # write: the control appears to run, playback continues, and the
            # result looks like "decode happens either way".
            #
            # `default` still starts the decoder enabled, so the positive path
            # needs no setup. It just also permits the experiment that gives
            # the positive result meaning.
            "media.hardware-video-decoding-vulkan.enabled" =
              { Value = true; Status = "default"; };
            # Direct export OFF.
            #
            # This pref was set true to fix the green frame, on the reasoning
            # that the GPU-copy path was the broken one and direct export
            # avoided its blit. The first half of that still holds; the second
            # half did not survive measurement.
            #
            # Direct export is not reachable on this hardware with an
            # unmodified Firefox. Firefox only requests the modifier-tiled
            # export shape -- it switches the frames pool to
            # DRM_FORMAT_MODIFIER_EXT only when the modifier list is not
            # linear-only, and calls av_hwframe_map only when the pool carries
            # that tiling. That exact combination, NV12 with
            # VIDEO_DECODE_DST and a DMA_BUF handle type, is refused by the
            # host NVIDIA driver itself, not merely by Venus: measured with
            # the same probe source on both stacks
            # (docs/dmabuf-export-finding.md). NV12 offers one modifier there,
            # LINEAR, without VIDEO_DECODE_OUTPUT.
            #
            # With the pref true, Firefox therefore built a frames pool whose
            # memory was never allocated exportable and mapped it anyway,
            # which crashed the RDD process. Firefox then relaunched RDD with
            # hardware decode disabled and decoded in software for the rest of
            # the session, silently -- the failure that took four coredumps to
            # attribute.
            #
            # Turning it off makes Firefox skip the modifier-tiling block
            # entirely, so av_hwframe_map is never called. That also avoids an
            # ffmpeg double-free in vulkan_map_to_drm's error path, which is
            # only reachable when the export fails.
            #
            # This selects a non-export route, not specifically the copy route.
            # Firefox has two non-export routes and picks between them on
            # whether HW_DECODED_VIDEO_ZERO_COPY is configured; the working
            # configuration gets zero copy. The copy route is still broken, and
            # separately so: its blit goes through the resource's own texture
            # rather than a sampler view, so it never reaches the per-plane
            # images that fix the chroma plane. That is in virglrenderer, which
            # this lab forks, and is recorded in SOLUTION.md section 6.
            "media.hardware-video-decoding-vulkan.direct-export.enabled" =
              { Value = false; Status = "default"; };
            # Also NOT locked, for the same reason and a measured one.
            #
            # force-enabled overrides the normal gating, so with it locked true
            # the negative control disabled the Vulkan pref, confirmed the write
            # took, and decode continued anyway: 512 commands and 3 sessions
            # with the feature nominally off. A control that cannot turn the
            # feature off is not a control, and this one was reporting a clean
            # pref write while the thing it was meant to disable kept running.
            #
            # It is force-enabled deliberately because the generic
            # HARDWARE_VIDEO_DECODING feature is blocklisted in the guest by a
            # VA-API probe that cannot succeed there. Keeping the default true
            # preserves that; unlocking it lets the control actually control.
            "media.hardware-video-decoding.force-enabled" =
              { Value = true; Status = "default"; };
            # NOTE: gfx.blacklist.hardwarevideodecoding is no longer set at all,
            # here or through extraPrefs. It existed to skip a VA-API probe the
            # guest could not pass; the guest passes it now. See the comment on
            # labFirefox.
            #
            # Pin zero-copy on rather than leaving it to the blocklist.
            # 0 forces off, 1 forces on, anything else defers to gfxInfo.
            # This one has an int default, so the policy sets it as an int.
            "media.ffmpeg.vaapi.force-surface-zero-copy" =
              { Value = 1; Status = "default"; };
            # Pin YouTube to H.264.
            "media.mediasource.webm.enabled" =
              { Value = false; Status = "locked"; };
            "media.webm.enabled" =
              { Value = false; Status = "locked"; };
          };
        };
      };

      # ---------------------------------------------------------------------
      # Guest image.
      # ---------------------------------------------------------------------
      guestSystem = nixpkgs.lib.nixosSystem {
        inherit system;
        specialArgs = { inherit labMesa labFirefox; };
        modules = [
          ./guest/configuration.nix
          ({ modulesPath, ... }: {
            imports = [ "${modulesPath}/profiles/qemu-guest.nix" ];
          })
        ];
      };

      guestImage = import "${nixpkgs}/nixos/lib/make-disk-image.nix" {
        inherit (guestSystem) config;
        inherit (guestSystem) pkgs;
        lib = nixpkgs.lib;
        format = "raw";
        diskSize = 16384;
        partitionTableType = "none";
        installBootLoader = false;
      };
    in
    {
      packages.${system} = {
        inherit labVirglrenderer labCrosvm labMesa labFirefox guestImage labCloudHypervisor;
        guestKernel = guestSystem.config.boot.kernelPackages.kernel;
        guestInitrd = guestSystem.config.system.build.initialRamdisk;
        default = labCrosvm;
      };

      # Kept explicit so `nix flake show` documents the split, and so anything
      # consuming this lab has to name which side it means.
      legacyPackages.${system} = { inherit hostPkgs guestPkgs; };

      apps.${system} = {
        # The single reproducible command that starts the lab VM using the
        # prototype packages. Supplies every runtime dependency and the built
        # image/kernel paths so the launcher needs no manual environment setup.
        lab-vm = {
          type = "app";
          program = "${hostPkgs.writeShellScript "lab-vm" ''
            set -euo pipefail
            export PATH=${hostPkgs.lib.makeBinPath [
              labCrosvm
              hostPkgs.passt
              hostPkgs.cage
              hostPkgs.bubblewrap
              labCloudHypervisor
              hostPkgs.qemu-utils
              hostPkgs.coreutils
              hostPkgs.gnused
              hostPkgs.gawk
              hostPkgs.gnugrep
              hostPkgs.procps
              hostPkgs.acl
            ]}:$PATH
            export VENUS_LAB_GRANT_KVM="${./host/grant-kvm.sh}"
            export VENUS_LAB_IMAGE="${guestImage}/nixos.img"
            export VENUS_LAB_KERNEL="${guestSystem.config.system.build.kernel}/bzImage"
            export VENUS_LAB_INITRD="${guestSystem.config.system.build.initialRamdisk}/initrd"
            export VENUS_LAB_INIT="${guestSystem.config.system.build.toplevel}/init"
            exec bash ${./host/run-lab-vm.sh} "$@"
          ''}";
        };

        # Runs a command inside the booted lab guest. This is the guest control
        # channel: without it every guest observation needs a predefined
        # systemd unit and therefore a full image rebuild.
        lab-ssh = {
          type = "app";
          program = "${hostPkgs.writeShellScript "lab-ssh" ''
            set -euo pipefail
            export PATH=${hostPkgs.lib.makeBinPath [
              hostPkgs.openssh hostPkgs.sshpass hostPkgs.coreutils
            ]}:$PATH
            exec bash ${./host/lab-ssh.sh} "$@"
          ''}";
        };

        # Reads the two Firefox gates from the LIVE cage session over
        # Marionette. These gate everything downstream: Firefox refuses
        # hardware decode unless it is already GPU-rendering. The W0 baseline
        # probed a --headless process, which never initialises WebRender, so it
        # could not observe either gate in any state.
        #
        # The probe is piped over stdin rather than read from the guest image,
        # so iterating on it costs one SSH round trip instead of a full image
        # rebuild and reboot.
        lab-firefox-gates = {
          type = "app";
          program = "${hostPkgs.writeShellScript "lab-firefox-gates" ''
            set -euo pipefail
            export PATH=${hostPkgs.lib.makeBinPath [
              hostPkgs.openssh hostPkgs.sshpass hostPkgs.coreutils
            ]}:$PATH
            exec bash ${./host/lab-ssh.sh} --stdin \
              "python3 - $*" < ${./guest/firefox-gates.py}
          ''}";
        };

        # Proves the crosvm <-> virglrenderer binding rather than assuming it.
        # Required by AGENTS.md rule 6.
        prove-crosvm-binding = {
          type = "app";
          program = "${hostPkgs.writeShellScript "prove-crosvm-binding" ''
            set -euo pipefail
            PATH=${hostPkgs.lib.makeBinPath [
              hostPkgs.coreutils hostPkgs.gnugrep hostPkgs.patchelf hostPkgs.nix
            ]}:$PATH

            crosvm=${labCrosvm}/bin/crosvm
            want=${labVirglrenderer}

            echo "lab crosvm:         $crosvm"
            echo "lab virglrenderer:  $want"
            echo

            echo "--- 1. --gpu-device-node flag present? ---"
            if "$crosvm" device gpu --help 2>&1 | grep -q -- "--gpu-device-node"; then
              echo "  PASS: flag present (crosvm-gpu-device-node.patch applied)"
            else
              echo "  FAIL: flag missing -- the gpu-device-node patch did not apply" >&2
              exit 1
            fi
            echo

            echo "--- 2. does the closure reference the LAB virglrenderer? ---"
            if nix-store --query --references "$(readlink -f "$crosvm")" 2>/dev/null \
                 | grep -q "^$want$"; then
              echo "  PASS: direct store reference to lab virglrenderer"
            elif patchelf --print-rpath "$crosvm" 2>/dev/null | grep -q "$want"; then
              echo "  PASS: RPATH points at lab virglrenderer"
            else
              echo "  FAIL: lab virglrenderer not referenced." >&2
              echo "  This is the symlinkJoin trap: crosvm is still resolving" >&2
              echo "  nixpkgs' stock virglrenderer through its RPATH." >&2
              echo "  rpath was:" >&2
              patchelf --print-rpath "$crosvm" 2>/dev/null | tr ':' '\n' | sed 's/^/    /' >&2
              exit 1
            fi
            echo
            echo "RESULT: crosvm is bound to the lab virglrenderer"
          ''}";
        };

        # Protocol checks against the venus-protocol fork: the append-only
        # command-id ABI gate and the H.264 flag wire-packing tests. Both are
        # wire contract, so both are runnable rather than one-off.
        protocol-checks = {
          type = "app";
          program = "${hostPkgs.writeShellScript "protocol-checks" ''
            set -euo pipefail
            export PATH=${hostPkgs.lib.makeBinPath [
              hostPkgs.coreutils hostPkgs.gnugrep hostPkgs.gnused hostPkgs.gcc
              hostPkgs.meson hostPkgs.ninja hostPkgs.pkg-config hostPkgs.git
              (hostPkgs.python3.withPackages (ps: [ ps.mako ]))
            ]}:$PATH
            export VENUS_LAB_PYTHON=python3

            # Both the source and the golden snapshot come from the flake, not
            # from a mutable working clone or an untracked state file, so this
            # validates exactly the revision the lock pins. The generator writes
            # nothing, but it needs the tree writable for Python bytecode, so
            # the read-only store copy is staged into a temp dir.
            tmp=$(mktemp -d)
            trap 'rm -rf "$tmp"' EXIT
            cp -r --no-preserve=mode,ownership ${venus-protocol-src} "$tmp/vp"
            vp="$tmp/vp"
            golden=${./tests/cmdids-golden.txt}

            echo "venus-protocol: ${venus-protocol-src}"
            echo

            # Run first: if the pin manifest is stale, every measurement the
            # rest of this script produces is attributed to the wrong revision.
            echo "--- PINS.md vs flake.lock ---"
            bash ${./tests/pins-check.sh} ${./.}

            echo
            echo "--- renderer video exposure ---"
            # "Advertise nothing" made mechanical. W3 flips this deliberately,
            # by deleting the gate in the same commit that adds the
            # completeness check replacing it.
            bash ${./tests/video-exposure-gate.sh} ${virglrenderer-src}

            echo
            echo "--- Venus capset video advertisement ---"
            # A separate advertisement channel from the extension table: the
            # guest reads the capset before issuing any command, so the
            # exposure gate cannot see this one.
            CAPSET_CLEAR_CHECK=${./tests/capset-clear-check.py} \
            bash ${./tests/video-capset-gate.sh} ${virglrenderer-src}

            echo
            echo "--- video value surface (E3/E5/E6) ---"
            # Every video-tagged enum value, bucketed and given a direction.
            # Seven separate "doors" were found by hand over four panel rounds;
            # four of them were the same thing seen four times. This derives the
            # whole set instead, and fails closed on anything it cannot classify.
            VENUS_PROTOCOL_DIR="$vp" \
              bash ${./tests/video-value-surface.sh} --check \
                ${./tests/video-value-surface-golden.txt}

            echo
            echo "--- video carrying-site manifest (E3 per-site) ---"
            # Which struct member, in which direction, can carry a video value.
            # The value surface says a value exists; this says where it
            # arrives, which is what an implementation actually has to handle.
            VENUS_PROTOCOL_DIR="$vp" \
              bash ${./tests/video-site-manifest.sh} --check \
                ${./tests/video-site-manifest-golden.txt}

            echo
            echo "--- video enforcement coverage ---"
            # The manifest proves classification; this proves enforcement.
            # 95 of 189 enforced, 93 gated, 1 deferred. Credit is per command
            # path: a row counts only when EVERY dispatched command that can
            # carry the value is guarded, because 48 rows name more than one.
            VIRGL_DIR=${virglrenderer-src} \
            VIDEO_SITE_MANIFEST=${./tests/video-site-manifest-golden.txt} \
              bash ${./tests/video-enforcement-gate.sh} --expect-unenforced 89

            echo
            echo "--- generated rejection table vs generator ---"
            # A stale generated header is indistinguishable from a current one
            # to the enforcement gate, which matches on type and member NAMES:
            # a helper whose value mask has fallen behind still reads as
            # enforced. Only regeneration separates "a check exists" from "the
            # check is the right one".
            VIRGL_DIR=${virglrenderer-src} \
            VENUS_PROTOCOL_DIR=${venus-protocol-src} \
            VIDEO_SITE_MANIFEST=${./tests/video-site-manifest-golden.txt} \
            VIDEO_REJECT_GENERATOR=${./tests/gen-video-reject.py} \
            VENUS_PROTOCOL_DIR=${venus-protocol-src} \
              bash ${./tests/generator-drift-check.sh}

            echo
            echo "--- dispatched commands outside the manifest ---"
            # The manifest is derived from vk.xml, but the renderer also
            # dispatches MESA vendor commands vk.xml does not describe. Those
            # surfaces are invisible rather than unguarded: there is no row for
            # any other gate to miss. Pinned at 2 -- both MESA host-copy
            # commands, both guarded by hand. A third would mean a new surface
            # nothing else can see.
            VIRGL_DIR=${virglrenderer-src} \
            VIDEO_SITE_MANIFEST=${./tests/video-site-manifest-golden.txt} \
              bash ${./tests/uncovered-dispatch-gate.sh} --expect-uncovered 2

            echo
            echo "--- rejected replies ---"
            # Rejecting a value and returning a well-formed, non-disclosing
            # reply are separate obligations. Every other gate here checks the
            # first; this one checks the second, after four defects across
            # three panel rounds came from the gap -- a zeroed sType tripping
            # an encoder assert, a nulled count pointer, unwritten output
            # payloads serialised from stale reply storage, and an unfiltered
            # capacity count.
            VIRGL_DIR=${virglrenderer-src} \
              bash ${./tests/reply-hygiene-gate.sh} --expect-unsanitized 0

            echo
            echo "--- video pNext surface (E5) ---"
            # The set of non-video entry points that decode a guest-supplied
            # video struct and pass it to the host. Derived from the generated
            # renderer, because hand-listing this surface was wrong twice.
            VENUS_PROTOCOL_DIR="$vp" \
              bash ${./tests/video-pnext-surface.sh} --check \
                ${./tests/video-pnext-surface-golden.txt}

            echo
            echo "--- video array cap audit ---"
            # Enumerates every video-reachable allocation rather than trusting
            # an audit by eye. Two hand audits both missed arrays.
            VENUS_PROTOCOL_DIR="$vp" bash ${./tests/video-array-cap-audit.sh}

            echo
            echo "--- vendored headers vs pinned generator ---"
            # The forks vendor generated headers instead of running the
            # generator, so drift here means the driver and renderer disagree
            # about the wire while every other check stays green.
            VENUS_PROTOCOL_DIR="$vp" \
            VIRGL_DIR=${virglrenderer-src} \
            MESA_DIR=${mesa-src} \
              bash ${./tests/header-sync-check.sh}

            echo
            echo "--- append-only command-id ABI gate ---"
            VENUS_PROTOCOL_DIR="$vp" \
            VENUS_PROTOCOL_BASE_DIR=${venus-protocol-base} \
            VENUS_ABI_LAYOUT_GOLDEN=${./tests/cmdids-golden-layout.txt} \
              bash ${./tests/abi-gate.sh} --check "$golden"

            echo
            echo "--- H.264 flag wire packing ---"
            gcc -std=c11 -Wall -Wextra -Werror -I "$vp/include" \
                -o "$tmp/flags-test" "$vp/tests/video_h264_flags.c"
            "$tmp/flags-test"

            # Executable round-trip and malformed-input tests. These need the
            # generated protocol headers, so the fork is configured and built
            # here rather than compiled ad hoc. meson runs the binary under
            # ASan/UBSan with halt_on_error, which is what makes the
            # truncation and corruption sweeps meaningful.
            echo
            echo "--- H.264 round-trip, truncation and corruption ---"
            # Pin the compiler. The host environment exports CC as an sccache
            # wrapper, and meson picks that up, so a broken sccache state fails
            # this gate with "Compiler cannot compile programs" while nothing
            # in the lab is wrong. The gate is supposed to be hermetic from the
            # lock; inheriting the host's compiler-cache wrapper is a leak.
            CC=gcc CXX=g++ meson setup "$tmp/build" "$vp"
            ninja -C "$tmp/build"
            meson test -C "$tmp/build" --print-errorlogs --suite venus-protocol
            sed -n '/^T1/,/failures$/p' "$tmp/build/meson-logs/testlog.txt"

            echo
            echo "--- scrub and rejection controls ---"
            # Every case here carries a control. Asserting the guest sees no
            # video bit passes whether or not scrubbing exists, because zero is
            # also an unset host bit; the positive control asserts the fixture
            # CARRIES the bit first. The negative controls catch the mirror
            # failure, where a reject function closes every door.
            ctl="$tmp/scrub"
            mkdir -p "$ctl"
            cp ${./tests/scrub/shim/vkr_common.h} "$ctl/vkr_common.h"
            cp ${virglrenderer-src}/src/venus/vkr_video_scrub.h "$ctl/"
            cp ${virglrenderer-src}/src/venus/vkr_video_reject.h "$ctl/"
            cp ${./tests/scrub/scrub-controls.c} "$ctl/scrub-controls.c"
            ( cd "$ctl" && gcc -std=c11 -Wall -Wextra -Werror \
                -I ${hostPkgs.vulkan-headers}/include \
                -fsanitize=address,undefined \
                -o scrub-controls scrub-controls.c && ./scrub-controls )

            # Negative controls for the W3 validators.
            #
            # Separate from scrub-controls because they test the opposite
            # polarity: scrub-controls proves rejections reject, these prove
            # validations do not silently accept everything. A no-op validator
            # is invisible to a positive control, so it needs its own harness
            # with inputs it must refuse.
            cp ${virglrenderer-src}/src/venus/vkr_video.h "$ctl/vkr_video.h"
            cp ${virglrenderer-src}/src/venus/vkr_video_validate.h "$ctl/vkr_video_validate.h"
            cp ${./tests/scrub/video-validate-controls.c} "$ctl/video-validate-controls.c"
            ( cd "$ctl" && gcc -std=c11 -Wall -Wextra -Werror \
                -I ${hostPkgs.vulkan-headers}/include \
                -fsanitize=address,undefined \
                -o video-validate-controls video-validate-controls.c \
              && ./video-validate-controls )
          ''}";
        };

        # Host-native Vulkan decode control. Pins the SAME ffmpeg the guest
        # uses so the host-vs-guest comparison is a real controlled experiment
        # rather than two different builds being compared.
        host-decode = {
          type = "app";
          program = "${hostPkgs.writeShellScript "host-decode" ''
            set -euo pipefail
            export PATH=${hostPkgs.lib.makeBinPath [
              hostPkgs.coreutils hostPkgs.gnugrep hostPkgs.gawk hostPkgs.ffmpeg
            ]}:$PATH
            export VENUS_LAB_FFMPEG="${hostPkgs.ffmpeg}/bin/ffmpeg"
            exec bash ${./tests/host-decode.sh} "$@"
          ''}";
        };

        # Proves the guest-side ICD is the LAB Mesa, and records the current
        # video baseline. Required by AGENTS.md rule 6.
        #
        # This catches "I added lab Mesa to the guest image, therefore the guest
        # uses it" -- which is false when VK_DRIVER_FILES still resolves to
        # /run/opengl-driver's stock Mesa.
        prove-guest-icd = {
          type = "app";
          program = "${hostPkgs.writeShellScript "prove-guest-icd" ''
            set -euo pipefail
            PATH=${hostPkgs.lib.makeBinPath [
              hostPkgs.coreutils hostPkgs.gnugrep hostPkgs.jq hostPkgs.gawk
            ]}:$PATH

            mesa=${labMesa}
            icd="$mesa/share/vulkan/icd.d/virtio_icd.x86_64.json"

            echo "lab mesa: $mesa"
            echo

            echo "--- 1. Venus (virtio) ICD present? ---"
            if [ ! -f "$icd" ]; then
              echo "  FAIL: no virtio_icd.x86_64.json in the lab Mesa" >&2
              exit 1
            fi
            lib=$(jq -r '.ICD.library_path' "$icd")
            api=$(jq -r '.ICD.api_version' "$icd")
            echo "  ICD:         $icd"
            echo "  library:     $lib"
            echo "  api_version: $api"
            echo

            echo "--- 2. does the ICD point INTO the lab Mesa? ---"
            case "$lib" in
              "$mesa"/*)
                echo "  PASS: library_path resolves inside the lab Mesa store path" ;;
              *)
                echo "  FAIL: library_path points outside the lab Mesa;" >&2
                echo "  the guest would be using a different Mesa than intended." >&2
                exit 1 ;;
            esac
            [ -f "$lib" ] || { echo "  FAIL: $lib does not exist" >&2; exit 1; }
            echo

            echo "--- 3. video extension baseline ---"
            # NOTE: do NOT use `strings` on libvulkan_virtio.so here. Every
            # Vulkan extension NAME appears in Mesa's common extension-name
            # table regardless of driver support, so a string match reports
            # PRESENT even when Venus advertises nothing. That is a false pass.
            #
            # The real signal is Venus's static passthrough allowlist: an
            # extension is only advertised to the guest if it appears in
            # vn_physical_device_get_passthrough_extensions() AND the renderer
            # also reports it. Inspect the source, not the binary.
            src=${mesa-src}
            vnpd="$src/src/virtio/vulkan/vn_physical_device.c"
            if [ ! -f "$vnpd" ]; then
              echo "  FAIL: cannot find vn_physical_device.c in the Mesa source" >&2
              exit 1
            fi

            passthrough=$(awk '
              /vn_physical_device_get_passthrough_extensions/ { infn = 1 }
              infn { print }
              infn && /^}/ { exit }
            ' "$vnpd")

            found=0
            for ext in KHR_video_queue KHR_video_decode_queue KHR_video_decode_h264; do
              if printf '%s\n' "$passthrough" | grep -c "\.$ext = true" >/dev/null; then
                echo "  $ext: in passthrough table"
                found=$((found + 1))
              else
                echo "  $ext: NOT in passthrough table (expected at W0 baseline)"
              fi
            done

            # The other half of W4: NV12 video format-feature bits are actively
            # stripped by MR !35842 and must be restored for decode to work.
            if grep -c "allowed_ycbcr_feats" "$vnpd" >/dev/null 2>&1; then
              ycbcr=$(awk '/allowed_ycbcr_feats =/,/;/' "$vnpd")
              if printf '%s\n' "$ycbcr" | grep -ci "VIDEO_DECODE" >/dev/null; then
                echo "  NV12 video format-feature bits: allowed"
              else
                echo "  NV12 video format-feature bits: STRIPPED (MR !35842, expected at W0 baseline)"
              fi
            fi
            echo
            if [ "$found" -eq 0 ]; then
              echo "RESULT: lab Mesa Venus ICD confirmed; video NOT exposed (W0 baseline)"
            else
              echo "RESULT: lab Mesa Venus ICD confirmed; video exposure DETECTED ($found/3)"
            fi
          ''}";
        };
      };

      checks.${system} = {
        inherit labVirglrenderer labCrosvm;
      };
    };
}
