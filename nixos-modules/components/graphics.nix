# Graphics support for nixling VMs (virtio-gpu + Wayland cross-domain
# forward to the host compositor). Imported by host.nix whenever a VM
# sets `nixling.vms.<name>.graphics.enable = true`.
#
# Hypervisor: cloud-hypervisor (chosen over crosvm because crosvm has
# no swtpm backend — its only `--vtpm-proxy` flag wires to ChromeOS's
# D-Bus vtpmd, useless outside ChromeOS). Cloud-hypervisor has native
# `--tpm socket=` and under microvm.nix uses the same
# `crosvm device gpu` sidecar over vhost-user-gpu that crosvm itself
# does, so Wayland cross-domain forwarding to the host compositor is
# unaffected by the swap.
{ lib, pkgs, config, ... }:

let
  # Our vendored, spectrum-os-patched cloud-hypervisor build (see
  # modules/nixling/ext/spectrum-ch/). Pulled into a let-binding so
  # the W2 assertion below can read `passthru.testedWithCrosvmRev`
  # and compare to `pkgs.crosvm.src.rev`.
  spectrumCH = import ../../pkgs/spectrum-ch { inherit pkgs; };

  # Patched wayland-proxy-virtwl that forwards EVERY host `wl_output`
  # global to the guest, not just the first one. Upstream
  # talex5/wayland-proxy-virtwl's relay.ml builds a single-entry-per-
  # interface registry, so the guest only ever sees one virtual
  # monitor regardless of how many physical monitors the host has —
  # which breaks SDL3 `/list:monitor`, FreeRDP `/multimon`, and
  # anything else that does Wayland-based monitor enumeration. The
  # patch special-cases `wl_output` to forward every host instance
  # through the existing per-entry-host_name machinery. See
  # patches/wayland-proxy-virtwl-multimon.patch for the diff.
  #
  # Upstream candidate: consider PR'ing this to
  # https://github.com/talex5/wayland-proxy-virtwl — the fix is
  # ~20 lines and obviously correct (multi-instance globals are
  # standard Wayland).
  waylandProxyVirtwlMultiMon = pkgs.wayland-proxy-virtwl.overrideAttrs (old: {
    patches = (old.patches or [ ]) ++ [
      ../../pkgs/patches/wayland-proxy-virtwl-multimon.patch
    ];
  });

  # The GPU sidecar is `crosvm device gpu`, spawned by microvm.nix's
  # cloud-hypervisor runner over vhost-user-gpu. Use the crosvm
  # nixpkgs ships (Feb 2026, rev 4c80bf3) directly — that rev speaks
  # the standardised vhost-user shmem message numbers
  # (`GET_SHMEM_CONFIG = 44`, `SHMEM_MAP = 9`, `SHMEM_UNMAP = 10`)
  # which match rust-vmm/vhost @ vhost-user-backend-v0.22.0 (the
  # vhost crate we use in modules/nixling/ext/spectrum-ch/ for
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
  };

  # security-r8-audio-11: port of talex5/crosvm@993b8e756 "Don't open
  # a graphical console window" to the vhost-user-gpu backend. See
  # the patch file for the full rationale. Without this, every
  # graphics VM start creates a chromeless, transparent, undecorated
  # window titled "crosvm" on the host compositor.
  crosvmNoGraphicalConsole = pkgs.crosvm.overrideAttrs (old: {
    patches = (old.patches or [ ]) ++ [
      ../../pkgs/patches/crosvm-no-graphical-console.patch
    ];
  });

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

{
  options.nixling.graphics.crossDomainTrusted = lib.mkOption {
    type = lib.types.bool;
    default = false;
    description = "Allow cross-domain Wayland forwarding via virtio-gpu for this VM. Default false; set true only for VMs where cross-domain is the primary use case (e.g. a Wayland-forwarding launchpad VM that runs FreeRDP or another remote-desktop client). Must be false for VMs running Docker.";
  };

  config = {
    # W2: build-time guard that the CH↔crosvm rev pair this module was
    # QA'd against hasn't drifted underneath us. If nixpkgs bumps crosvm
    # independently of our vendored CH (which carries the matching
    # vhost-user-gpu wire protocol expectations), this assertion forces
    # a manual re-test before the system will evaluate.
    assertions = [
      {
        assertion = spectrumCH.passthru.testedWithCrosvmRev == pkgs.crosvm.src.rev;
        message = "CH and crosvm rev pair drifted — review compatibility first. spectrum-ch.testedWithCrosvmRev=${spectrumCH.passthru.testedWithCrosvmRev}, crosvm.src.rev=${pkgs.crosvm.src.rev}";
      }
    ];

    microvm = {
      # mkDefault so tpm.nix (which also sets cloud-hypervisor) doesn't
      # produce a duplicate-definition error when both modules are
      # imported.
      hypervisor = lib.mkDefault "cloud-hypervisor";

      # Suppress fbcon binding to virtio-gpu in the guest. The GPU sidecar
      # is forced to use the Wayland display backend (the unnamed wayland
      # socket is structurally tied to the cross-domain channel, so we
      # can't escape Wayland-as-display-backend), but the host scanout
      # window only becomes visible once the guest issues a virtio-gpu
      # SET_SCANOUT command. fbcon does that when it binds to the
      # framebuffer; aim it at fb99 (which doesn't exist) and the kernel
      # console stays on ttyS0/serial only — no SET_SCANOUT, no host
      # scanout window. Cross-domain Wayland forwarding (foot, Firefox)
      # uses a separate virtio-gpu opcode path and is unaffected.
      kernelParams = [ "nofb" "video=off" ];

      graphics.enable = true;

      # microvm.nix's cloud-hypervisor runner uses `crosvm device gpu` as
      # the GPU sidecar over vhost-user-gpu. Feed nixpkgs's crosvm
      # directly (see crosvmPatched let-binding for why no overrides).
      #
      # P5 W3: gate the cross-domain Wayland context type on
      # `nixling.graphics.crossDomainTrusted`. When the option is false
      # (the default — set true only for VMs that legitimately need
      # cross-domain Wayland forwarding, e.g. a Wayland-forwarding
      # launchpad VM running FreeRDP), wrap crosvm in a shell shim
      # that strips `cross-domain` from the `--params` JSON before
      # invoking the real binary. Stripping is tolerant of the three
      # syntactic shapes microvm.nix's generator can emit
      # (`context-types=cross-domain:virgl2`, `…:cross-domain`,
      # standalone `cross-domain`). The wrapped binary keeps all other
      # GPU capabilities (virgl2, etc.) so the VM still gets a
      # functioning virtio-gpu display.
      #
      # NOTE on the chromeless "crosvm" window:
      #
      # crosvm's gpu_display/src/display_wl.c unconditionally creates
      # an xdg_toplevel titled "crosvm" for every scanout surface;
      # `DisplayParameters.hidden` is only honored on Windows
      # (start_hidden in gpu_display_win/surface.rs), not Linux/
      # Wayland. KDE renders the window without decoration (crosvm
      # never requests xdg-decoration SSD) and shows the default 'W'
      # placeholder icon — observed on every VM start.
      #
      # An earlier attempt to patch display_wl.c to skip the
      # xdg_surface/xdg_toplevel creation when hidden=true broke the
      # virtio-gpu SetScanout flow: the guest's virtio_gpu driver
      # issues SetScanout regardless of fbcon, and crosvm needs a
      # configured xdg_surface for the scanout buffer to land on.
      # Skipping the xdg_surface produced `failed to find
      # parent_surface` + `ErrDisplay(CreateSurface)`.
      #
      # Mitigation lives in host.nix: a KWin window rule keyed on
      # title=^crosvm$ + empty resourceClass hides + skip-taskbars +
      # skip-pagers the window so it never appears in the user's
      # workspace. The scanout protocol stays intact; only the
      # visible artifact is suppressed.
      graphics.crosvmPackage =
        if config.nixling.graphics.crossDomainTrusted
        then crosvmPatched
        else
          let
            realCrosvm = crosvmPatched;
          in pkgs.writeShellScriptBin "crosvm" ''
            newargs=()
            while [ $# -gt 0 ]; do
              if [ "$1" = "--params" ] && [ $# -ge 2 ]; then
                stripped=$(printf '%s' "$2" \
                  | ${pkgs.gnused}/bin/sed \
                      -e 's/cross-domain://g' \
                      -e 's/:cross-domain//g' \
                      -e 's/cross-domain//g')
                newargs+=( "--params" "$stripped" )
                shift 2
              else
                newargs+=( "$1" )
                shift
              fi
            done
            exec ${realCrosvm}/bin/crosvm "''${newargs[@]}"
          '';

      # microvm.nix's option default for `cloud-hypervisor.package` is
      # `cfg.vmHostPackages.cloud-hypervisor-graphics`, a spectrum-os-
      # patched build that lives only in microvm.nix's own overlay.
      # That overlay depends on fetching spectrum-os.org's git tree,
      # whose snapshot tarball and git-over-http servers are both
      # broken (consistent truncation under 100KB, fetch-pack RST).
      #
      # Solution: we vendor the (tiny) patch set in
      # modules/nixling/ext/spectrum-ch/ and build the patched
      # cloud-hypervisor ourselves.
      cloud-hypervisor.package = spectrumCH;
    };

    hardware.graphics.enable = true;

    # Without monospace fonts installed, fontconfig's "monospace" alias
    # falls back to DejaVu Sans (proportional) and foot warns:
    #   "DejaVu Sans: font does not appear to be monospace"
    # dejavu_fonts ships DejaVu Sans Mono which fontconfig promotes to
    # the canonical monospace alias on resolution.
    fonts.packages = with pkgs; [
      dejavu_fonts
      liberation_ttf
      noto-fonts
    ];

    environment.sessionVariables = {
      WAYLAND_DISPLAY = "wayland-1";
      DISPLAY = ":0";
      QT_QPA_PLATFORM = "wayland";
      GDK_BACKEND = "wayland";
      XDG_SESSION_TYPE = "wayland";
      SDL_VIDEODRIVER = "wayland";
      CLUTTER_BACKEND = "wayland";
      MOZ_ENABLE_WAYLAND = "1";

      # Mesa ships a single monolithic package that registers ICDs for every
      # GPU driver (radv, anv, nouveau, nvk, panfrost, lvp, venus, ...).
      # The Vulkan loader probes ALL of them at process startup; in this VM
      # the only GPU is virtio-gpu so radv/amdgpu/etc. log noisy
      #   "failed to initialize device, could not get caps: Invalid argument"
      # errors when Chromium/Electron/Qt apps init Vulkan. Constrain probing
      # to the single ICD that matches our hardware.
      #
      # virtio_icd → venus (Vulkan-over-virtio, terminated on the host).
      # lvp_icd    → lavapipe (CPU software fallback) — kept so apps that
      #              insist on Vulkan never fail outright if venus gates
      #              on a missing host feature.
      VK_DRIVER_FILES = "/run/opengl-driver/share/vulkan/icd.d/virtio_icd.x86_64.json:/run/opengl-driver/share/vulkan/icd.d/lvp_icd.x86_64.json";

      # Same idea for GL: skip the "find a real card" probing phase and
      # tell Mesa's DRI loader directly that virtio_gpu is what to use.
      MESA_LOADER_DRIVER_OVERRIDE = "virtio_gpu";

      # Mesa 24+ ships zink (the Vulkan-on-GL layer) as a GL fallback
      # via the kopper loader. With virtio-gpu but no exposed Vulkan
      # device, SDL3's EGL init probes zink, can't pick a pdev, and
      # emits an MESA: error: ZINK: failed to choose pdev plus two
      # libEGL: failed to create dri2 screen warnings per connect.
      # Disabling kopper skips that probe entirely. libEGL still
      # succeeds via virgl's native virtio-gpu DRI path.
      LIBGL_KOPPER_DISABLE = "true";

      # libEGL's own probe failures (e.g. when SDL3 tries protocols
      # the virtio-gpu DRI driver doesn't expose) print warnings at
      # severity=warning. The fallbacks always work, so silence the
      # probe noise.
      EGL_LOG_LEVEL = "fatal";
    };

    systemd.user.services.wayland-proxy = {
      description = "Wayland Proxy (virtio-gpu cross-domain to host compositor)";
      wantedBy = [ "default.target" ];
      serviceConfig = {
        # waylandProxyVirtwlMultiMon = nixpkgs wayland-proxy-virtwl +
        # our multi-wl_output patch. Without the patch the guest only
        # sees one host monitor; see the let-binding above for context.
        #
        # --tag prefixes every guest window's title with the VM name
        # in square brackets (e.g. "[corp-desktop] Mozilla Firefox"),
        # so the operator can tell at a glance which VM a window
        # belongs to when multiple graphics VMs are running side-by-
        # side on the host. config.networking.hostName is set to the
        # VM name in the manifest (base.nix). wayland-proxy-virtwl's
        # --tag is prepended verbatim, so include the brackets and
        # trailing
        # space in the value.
        ExecStart = "${waylandProxyVirtwlMultiMon}/bin/wayland-proxy-virtwl --virtio-gpu --tag=[${config.networking.hostName}]\\  --x-display=0 --xwayland-binary=${pkgs.xwayland}/bin/Xwayland";
        Restart = "on-failure";
        RestartSec = 5;
      };
    };

    # security-r8-audio-8: in-guest foot-autostart removed.
    #
    # The original purpose was to give the VM a visible window on the
    # host's Plasma session as soon as it booted, because the
    # autologin getty renders to the in-guest fbcon (which crosvm's
    # cross-domain GPU context does NOT forward to the host).
    #
    # The problem: foot is a Wayland-native terminal that by design
    # does NOT implement CSD (https://codeberg.org/dnkl/foot/wiki/Frequently-asked-questions).
    # It relies on xdg-decoration for SSD from the compositor.
    # Inside the guest, foot's xdg-decoration request transits
    # wayland-proxy-virtwl → virtio-gpu cross-domain channel → host
    # KDE compositor, but Plasma's response (SSD-mode) doesn't reach
    # foot via that path in a usable way. Result: a chromeless
    # terminal pops up on every boot with no title bar, no close
    # button, and the default "W" icon (Plasma's unknown-app
    # placeholder).
    #
    # Fix: don't auto-launch a guest terminal at all. The per-VM
    # .desktop entry (cli.nix `desktopItems`) now launches a HOST-
    # side foot terminal that SSHes into the VM, which gets proper
    # chrome from the host KDE compositor via standard
    # xdg-decoration. Operators can still SSH from any other host
    # terminal at any time — the launcher is just one convenience
    # path.
  };
}
