# Guest NixOS configuration for the Venus Vulkan Video lab.
#
# Deliberately minimal: a Wayland session running Firefox on virtio-gpu, plus
# the tools needed to produce capability and decode evidence. This is a lab
# guest, not a d2b VM -- it has no d2b agent, no guest-control, and no realm
# membership.
{ lib, pkgs, labMesa, labFirefox, ... }:

{
  system.stateVersion = "25.11";

  # --- Boot / disk ---------------------------------------------------------
  # Plain virtio-blk root. Cloud Hypervisor boots the kernel directly, so there
  # is no bootloader in the image.
  boot.loader.grub.enable = false;
  boot.initrd.availableKernelModules = [
    "virtio_pci" "virtio_blk" "virtio_net" "virtio_console" "virtio_gpu"
  ];
  boot.kernelModules = [ "virtio_gpu" ];
  boot.kernelParams = [ "console=ttyS0" ];

  fileSystems."/" = {
    device = "/dev/vda";
    fsType = "ext4";
    autoResize = true;
  };

  # --- Graphics ------------------------------------------------------------
  # THE load-bearing part of this file.
  #
  # Pointing hardware.graphics.package at the lab Mesa is what puts the lab's
  # Venus ICD into /run/opengl-driver. Merely adding labMesa to
  # environment.systemPackages would NOT do this: the Vulkan loader reads
  # /run/opengl-driver/share/vulkan/icd.d, so the guest would silently keep
  # using stock Mesa. `nix run .#prove-guest-icd` guards the package itself;
  # `tests/guest-caps.sh` re-proves it from inside the booted guest.
  hardware.graphics = {
    enable = true;
    package = labMesa;
  };

  environment.sessionVariables = {
    # Constrain ICD probing to Venus (the only real GPU here) plus lavapipe as
    # a software fallback, so Vulkan apps do not emit noise for radv/anv/etc.
    # Resolved through /run/opengl-driver, which hardware.graphics.package
    # above populates from the LAB Mesa.
    VK_DRIVER_FILES = lib.concatStringsSep ":" [
      "/run/opengl-driver/share/vulkan/icd.d/virtio_icd.x86_64.json"
      "/run/opengl-driver/share/vulkan/icd.d/lvp_icd.x86_64.json"
    ];
    MESA_LOADER_DRIVER_OVERRIDE = "virtio_gpu";
    LIBGL_KOPPER_DISABLE = "true";
    EGL_LOG_LEVEL = "fatal";

    MOZ_ENABLE_WAYLAND = "1";
    XDG_SESSION_TYPE = "wayland";
    GDK_BACKEND = "wayland";
  };

  # --- Session -------------------------------------------------------------
  # cage runs a single fullscreen Wayland client on the virtio-gpu scanout,
  # which crosvm forwards to the host window. Firefox is that client, so the
  # browser is GPU-rendered through the same virtio-gpu device that will carry
  # decode -- the "same physical GPU" invariant the prototype is about.
  #
  # Marionette is enabled on the CAGE Firefox specifically. The W0 baseline
  # probed a --headless Firefox instead, which never initialises WebRender at
  # all, so it reported zero WebRender and zero Vulkan mentions and proved
  # nothing about the gates that actually block hardware decode. The gates
  # (LAYERS_WR && !UsingSoftwareWebRender, and gfxVars::UseH264HwDecode) are
  # properties of the real GPU-rendered session, so they must be read from it.
  services.cage = {
    enable = true;
    user = "lab";
    program = "${pkgs.writeShellScript "lab-firefox-session" ''
      # -remote-allow-system-access is required for Marionette chrome context.
      # Without it the connection succeeds and NewSession succeeds, and only
      # Marionette:SetContext fails -- so a probe that does not check every
      # reply looks like it is working while reading nothing.
      exec ${labFirefox}/bin/firefox --marionette -remote-allow-system-access "$@"
    ''}";
  };

  # Capture Firefox's own account of the Vulkan decode -> compositor handoff.
  #
  # The renderer counters prove vkCmdDecodeVideoKHR ran. They cannot show which
  # DRM modifier was negotiated, whether direct export or the copy path was
  # taken, or what the resulting DMA-BUF descriptor looked like -- and that
  # handoff is where the frames are being lost. Firefox logs all of it under
  # PlatformDecoderModule/FFmpegVideo; this is the only place that information
  # exists.
  systemd.services."cage-tty1".environment = {
    MOZ_LOG = "PlatformDecoderModule:5,FFmpegVideo:5,DMABUFSurface:5,Dmabuf:5";
    MOZ_LOG_FILE = "/tmp/ff-vulkan.log";

    # Guest-side dmabuf import trace, from the lab Mesa.
    #
    # This is a GUEST variable. The host passthrough list in run-lab-vm.sh
    # covers the renderer's own traces; this one is emitted by Mesa inside the
    # VM, so it has to be set on the process that runs Firefox.
    VIRGL_TRACE_IMPORT = "1";

    # Make libavcodec.so.62 (ffmpeg 8) resolvable so Firefox does not fall
    # back to libavcodec.so.61 (ffmpeg 7).
    #
    # Firefox already PREFERS 62: FFMPEG_MAX_MAJOR_VERSION is 62 and its
    # dlopen list in FFmpegRuntimeLinker.cpp tries "libavcodec.so.62" before
    # "libavcodec.so.61". It got 61 anyway because the nixpkgs wrapper
    # hardcodes ffmpeg_7 into the wrapped LD_LIBRARY_PATH, so 62 was simply
    # not on the path. The sonames differ, so adding ffmpeg 8's lib directory
    # is sufficient and no ordering games are needed -- dlopen of ".so.62"
    # cannot match anything in the ffmpeg 7 directory.
    #
    # This matters because ffmpeg 7's DMA-BUF export path is broken for
    # Vulkan video decode output. vulkan_map_to_drm() waits on the frame's
    # semaphores with
    #     semaphoreCount = av_pix_fmt_count_planes(sw_format)
    # while f->sem[] is sized by the image count. A decoded NV12 frame is ONE
    # multi-planar VkImage, so planes == 2 but nb_images == 1, and the wait
    # reads f->sem[1] == VK_NULL_HANDLE. Venus then dereferences the null
    # semaphore in vn_get_semaphore_counter_value and the RDD process dies
    # with SIGSEGV. Firefox restarts RDD, retries with hardware decode
    # disabled, and every later frame is decoded in software -- with no
    # message anywhere saying hardware decode was abandoned.
    #
    # ffmpeg 8 fixed exactly this: the wait now uses
    #     semaphoreCount = ff_vk_count_images(f)
    #
    # Verified by the crash moving off the semaphore path and onto
    # vn_GetMemoryFdKHR under FFmpegVideoDecoder<62> + libavutil.so.60.
    LD_LIBRARY_PATH = "${pkgs.ffmpeg.lib}/lib";

    # Report the guest virtio-gpu DRM node truthfully.
    #
    # Venus zeroes it on NVIDIA hosts so the WSI same-GPU check fails and the
    # prime-blit path is taken. That is a WSI workaround, but the zeroed node
    # is what Firefox reads to decide whether the decode device and the
    # compositor are the same GPU -- and when it decides they are not, it skips
    # its own NVIDIA workaround that substitutes a tiled DRM modifier for
    # LINEAR. LINEAR-only disables direct decode export, forcing a copy path
    # whose GL blit fails on virgl and wedges the rendering context.
    #
    # Safe here specifically because this guest composites through GL, so
    # Vulkan WSI presentation is not the path in use and the spoof protects
    # nothing.
    VN_DEBUG = "no_nvidia_drm_spoof";
  };

  users.users.lab = {
    isNormalUser = true;
    password = "lab";                 # lab guest, no secrets, no network exposure
    extraGroups = [ "video" "render" "wheel" ];
  };
  security.sudo.wheelNeedsPassword = false;
  services.getty.autologinUser = "lab";

  # --- Networking ----------------------------------------------------------
  # passt provides unprivileged NAT on the host side; the guest just needs DHCP.
  networking.hostName = "venus-lab";
  networking.useDHCP = lib.mkForce true;
  networking.firewall.enable = false;

  # --- Control channel -----------------------------------------------------
  # W0 shipped with serial-only management, so every guest observation needed a
  # predefined systemd service and therefore a full image rebuild. That is one
  # rebuild per experiment across the decode, Firefox and benchmark waves, and
  # it is the single largest avoidable cost in the remaining plan.
  #
  # Password auth on a lab-only account is deliberate and is not a new exposure:
  # the account password is already a literal in this file (and therefore in the
  # Nix store), and passt forwards the port from the host loopback only, so the
  # guest is not reachable from the network. A key pair would add key management
  # for no additional confidentiality here.
  services.openssh = {
    enable = true;
    settings = {
      PasswordAuthentication = true;
      PermitRootLogin = "no";
    };
  };

  # --- Evidence tooling ----------------------------------------------------
  environment.systemPackages = with pkgs; [
    vulkan-tools          # vulkaninfo: guest capability reports
    ffmpeg                # -hwaccel vulkan decode tests (built --enable-vulkan)
    strace                # prove no /dev/video* is opened
    mesa-demos
    pciutils
    jq
    python3               # marionette client (gfx/media gate probe)
    grim                  # screenshot the cage output -- see below
    labFirefox

    # Answers whether Venus can export a decode-output image as a DMA-BUF.
    #
    # Built here rather than compiled ad hoc and copied in, because it has to
    # link against the guest's own Vulkan headers and loader to mean anything.
    # A host-built binary would be probing the host stack, which is the exact
    # confusion this lab has already paid for once.
    (runCommandCC "venus-probe-dmabuf-export"
      { buildInputs = [ vulkan-headers vulkan-loader ]; }
      ''
        mkdir -p $out/bin
        $CC -O1 -Wall -o $out/bin/venus-probe-dmabuf-export \
          ${../tests/probe-dmabuf-export.c} -lvulkan
      '')
  ];

  # Screenshots are evidence, not decoration.
  #
  # Command-level proof that vkCmdDecodeVideoKHR executed says the decoder ran.
  # It says nothing about whether the decoded frames are CORRECT or reach the
  # screen intact, and those are different failures with different causes. A
  # green frame with a healthy decode-command count is exactly the shape that
  # slips through a counter-only evidence contract.


  # Reads the live Firefox session's own troubleshooting snapshot over
  # Marionette. This is the only way to get the real answer: the gates are
  # runtime gfxVars/gfxFeature state, not build config, and they differ between
  # a headless process and the GPU-rendered session.
  environment.etc."venus-lab/firefox-gates.py".source = ./firefox-gates.py;

  # Serial console is the only management path; there is no SSH and no d2b
  # guest agent in this image on purpose.
  systemd.services."serial-getty@ttyS0".enable = true;

  # --- Evidence capture ----------------------------------------------------
  # Emit the guest capability report to the serial console at boot, framed by
  # unambiguous markers. This makes the W0 baseline (and every later wave's
  # flip) reproducible without an interactive login, and it is the guest half
  # of the host-side tests/host-caps.sh probe.
  #
  # As on the host, capabilities are attributed to a SPECIFIC device: this
  # guest also exposes lavapipe, so a whole-file grep would report video
  # support that the Venus device does not actually have.
  systemd.services.venus-lab-caps = {
    description = "Venus lab guest capability report";
    wantedBy = [ "multi-user.target" ];
    after = [ "systemd-udev-settle.service" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      StandardOutput = "journal+console";
      StandardError = "journal+console";
    };
    script = ''
      set -u
      echo "===VENUS-LAB-CAPS-BEGIN==="

      echo "--- ICD in use ---"
      icd=/run/opengl-driver/share/vulkan/icd.d/virtio_icd.x86_64.json
      if [ -f "$icd" ]; then
        echo "icd_json=$(readlink -f "$icd")"
        echo "icd_lib=$(${pkgs.jq}/bin/jq -r '.ICD.library_path' "$icd")"
      else
        echo "icd_json=MISSING"
      fi

      report=/tmp/guest-vulkaninfo.txt
      if ! ${pkgs.vulkan-tools}/bin/vulkaninfo > "$report" 2>&1; then
        echo "vulkaninfo=FAILED"
        tail -5 "$report" || true
        echo "===VENUS-LAB-CAPS-END==="
        exit 0
      fi

      # Locate the Venus device block. Venus reports deviceName "Virtio-GPU
      # Venus (...)"; fall back to driverName virtio.
      start=$(${pkgs.gawk}/bin/awk '
        /^GPU[0-9]+:/ { g = NR }
        /deviceName/ && (/Venus/ || /Virtio/) { print g; exit }
      ' "$report")
      if [ -z "$start" ]; then
        echo "venus_device=NOT_FOUND"
        echo "devices_seen:"
        grep "deviceName" "$report" | sed 's/^/  /' || true
        echo "===VENUS-LAB-CAPS-END==="
        exit 0
      fi
      end=$(${pkgs.gawk}/bin/awk -v s="$start" 'NR>s && /^GPU[0-9]+:/ { print NR; exit }' "$report")
      [ -n "$end" ] || end=$(wc -l < "$report")

      block=$(sed -n "''${start},''${end}p" "$report")
      echo "--- Venus device block (lines $start-$end) ---"
      printf '%s\n' "$block" | grep -E "deviceName|driverName|driverInfo|apiVersion" | head -4

      echo "--- video capability ---"
      for ext in VK_KHR_video_queue VK_KHR_video_decode_queue VK_KHR_video_decode_h264; do
        if printf '%s\n' "$block" | grep -c -- "$ext" >/dev/null; then
          echo "$ext=PRESENT"
        else
          echo "$ext=absent"
        fi
      done
      if printf '%s\n' "$block" | grep -c "QUEUE_VIDEO_DECODE_BIT_KHR" >/dev/null; then
        echo "video_decode_queue_bit=PRESENT"
      else
        echo "video_decode_queue_bit=absent"
      fi

      echo "--- V4L2 / virtio-media (must stay absent) ---"
      echo "dev_video_nodes=$(ls /dev/video* 2>/dev/null | tr '\n' ' ' || true)"

      echo "===VENUS-LAB-CAPS-END==="
    '';
  };

  # Firefox decoder-selection baseline.
  #
  # Firefox's Vulkan Video fallback is SILENT: when no device advertises
  # VK_KHR_video_queue it logs one line and quietly drops to VA-API and then
  # software. So "the video played" proves nothing, and the absence of an error
  # proves nothing either. This service captures the decoder-selection evidence
  # explicitly, framed by markers, so the W0 baseline is an artifact rather than
  # an impression -- and so the same capture flips visibly once W4 lands.
  #
  # It drives a headless Firefox against a locally generated H.264 file. No
  # network, so the result cannot be perturbed by YouTube's adaptive bitrate,
  # ads, or cache.
  systemd.services.venus-lab-firefox-baseline = {
    description = "Venus lab Firefox decoder-selection baseline";
    wantedBy = [ "multi-user.target" ];
    after = [ "venus-lab-caps.service" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      User = "lab";
      StandardOutput = "journal+console";
      StandardError = "journal+console";
      WorkingDirectory = "/tmp";
    };
    environment = {
      MOZ_ENABLE_WAYLAND = "1";
      # Decoder selection + Vulkan device selection live in these modules.
      MOZ_LOG = "PlatformDecoderModule:5,FFmpegVideo:5,webrender:4,PlatformDecoderModule:5";
      MOZ_LOG_FILE = "/tmp/firefox-decoder.log";
      XDG_RUNTIME_DIR = "/run/user/1000";
    };
    script = ''
      set -u
      echo "===VENUS-LAB-FIREFOX-BEGIN==="

      # Deterministic local H.264 clip: 8-bit 4:2:0 progressive, the exact
      # profile the prototype targets. Generated rather than downloaded so the
      # baseline is reproducible offline.
      clip=/tmp/h264-baseline.mp4
      if ! ${pkgs.ffmpeg}/bin/ffmpeg -y -loglevel error \
             -f lavfi -i testsrc=size=1280x720:rate=30:duration=3 \
             -pix_fmt yuv420p -c:v libx264 -profile:v high "$clip" 2>&1; then
        echo "clip_generation=FAILED"
        echo "===VENUS-LAB-FIREFOX-END==="
        exit 0
      fi
      echo "clip=$clip ($(stat -c %s "$clip") bytes)"

      # Does the guest ffmpeg even offer a Vulkan hwaccel? If not, Firefox
      # could never use one either, and that would be the real finding.
      echo "ffmpeg_hwaccels=$(${pkgs.ffmpeg}/bin/ffmpeg -hide_banner -hwaccels 2>/dev/null \
        | tail -n +2 | tr '\n' ' ')"

      # Independent of Firefox: try a Vulkan decode directly.
      #
      # IMPORTANT: a zero exit from `ffmpeg -hwaccel vulkan ... -f null -` does
      # NOT prove Vulkan decode happened. ffmpeg falls back to software when
      # hwaccel init fails and still exits 0. The decisive evidence is in the
      # verbose log: whether a hwaccel was actually initialised, and what the
      # decoder ended up being.
      if ${pkgs.ffmpeg}/bin/ffmpeg -hide_banner -loglevel verbose -y \
           -hwaccel vulkan -hwaccel_output_format vulkan \
           -i "$clip" -f null - > /tmp/ffmpeg-vulkan.log 2>&1; then
        echo "ffmpeg_vulkan_exit=0"
      else
        echo "ffmpeg_vulkan_exit=nonzero"
      fi
      # Attribute the outcome rather than trusting the exit code.
      init=$(grep -ciE "Init(ialized)? .*vulkan|using vulkan|vulkan_decode" /tmp/ffmpeg-vulkan.log || true)
      fell=$(grep -ciE "Failed setup for format vulkan|falling back|not supported|No device available" /tmp/ffmpeg-vulkan.log || true)
      # Decisive signal: the decoder's OUTPUT pixel format. vulkan => frames are
      # Vulkan-backed; yuv420p => software decode regardless of what was asked.
      pixfmt=$(grep -oE "pix_fmt: [a-z0-9]+" /tmp/ffmpeg-vulkan.log | tail -1 | ${pkgs.gawk}/bin/awk '{print $2}' || true)
      echo "ffmpeg_vulkan_init_lines=$init"
      echo "ffmpeg_vulkan_fallback_lines=$fell"
      echo "ffmpeg_decoder_output_pix_fmt=''${pixfmt:-unknown}"
      if [ "''${pixfmt:-}" = "vulkan" ] && [ "$fell" -eq 0 ]; then
        echo "ffmpeg_vulkan_decode=USED_VULKAN"
      else
        echo "ffmpeg_vulkan_decode=DID_NOT_USE_VULKAN"
      fi
      echo "--- ffmpeg verbose (decoder/hwaccel lines) ---"
      grep -iE "vulkan|hwaccel|decoder|h264" /tmp/ffmpeg-vulkan.log 2>/dev/null \
        | head -8 | sed 's/^/  /' || echo "  (none)"

      rm -f /tmp/firefox-decoder.log*
      cat > /tmp/play.html <<'HTML'
<!doctype html><meta charset=utf-8>
<video id=v src="h264-baseline.mp4" autoplay muted></video>
<script>
  const v = document.getElementById('v');
  v.addEventListener('ended', () => { document.title = 'DONE'; });
  setTimeout(() => { document.title = 'DONE'; }, 20000);
</script>
HTML

      # Run long enough to actually exercise the decoder and flush MOZ_LOG.
      # `--screenshot` exits almost immediately -- often before the media stack
      # has selected a decoder at all -- which is why the first version of this
      # capture produced an empty log.
      prof=/tmp/ff-profile
      rm -rf "$prof"; mkdir -p "$prof"
      ${labFirefox}/bin/firefox --headless --profile "$prof" --no-remote \
        file:///tmp/play.html > /tmp/firefox-run.log 2>&1 &
      ff_pid=$!
      sleep 25
      kill "$ff_pid" 2>/dev/null || true
      # Give it a moment to flush the log before it is read.
      sleep 3
      kill -9 "$ff_pid" 2>/dev/null || true
      wait "$ff_pid" 2>/dev/null || true

      # Firefox writes ONE MOZ_LOG file PER PROCESS. Media decoding happens in a
      # child (RDD/content) process, so picking the first match found only an
      # empty child log and reported "0 lines" while the real content sat in a
      # sibling file. Concatenate them all.
      cat /tmp/firefox-decoder.log*.moz_log > /tmp/firefox-decoder.all 2>/dev/null || true
      log=/tmp/firefox-decoder.all
      nlines=$(wc -l < "$log" 2>/dev/null || echo 0)
      echo "firefox_log_files=$(ls /tmp/firefox-decoder.log*.moz_log 2>/dev/null | wc -l)"
      echo "firefox_log_lines=$nlines"
      if [ "$nlines" -eq 0 ]; then
        echo "firefox_log=EMPTY"
        tail -3 /tmp/firefox-run.log 2>/dev/null | sed 's/^/  run| /' || true
        echo "===VENUS-LAB-FIREFOX-END==="
        exit 0
      fi

      echo "--- decoder selection ---"
      # `grep -c` exits 1 on zero matches, so `|| echo 0` would print a SECOND
      # line. Capture the count and default it instead.
      for pat in Vulkan VAAPI V4L2 "software decoder" "No suitable"; do
        n=$(grep -ci -- "$pat" "$log" 2>/dev/null) || n=0
        echo "mentions[$pat]=$n"
      done


      # Firefox's rendering/codec gates.
      #
      # These are load-bearing: Firefox REFUSES hardware decode unless it is
      # already GPU-rendering (LAYERS_WR && !UsingSoftwareWebRender) and
      # UseH264HwDecode() is true. "GPU-decoded video" and "GPU-rendered
      # Firefox" are therefore not independent goals -- the second gates the
      # first -- so capturing them means a W6 failure can be attributed rather
      # than guessed at.
      echo "--- Firefox render/codec gates ---"
      for pat in "WebRender" "wr_" "SoftwareWebRender" "UseH264HwDecode" "gfxVars"; do
        n=$(grep -ci -- "$pat" "$log" 2>/dev/null) || n=0
        echo "gate_mentions[$pat]=$n"
      done
      grep -iE "webrender|compositor" "$log" 2>/dev/null | head -4 | sed 's/^/  /' \
        || echo "  (no WebRender lines captured)"

      echo "--- verbatim Vulkan lines ---"
      grep -i "vulkan" "$log" 2>/dev/null | head -5 | sed 's/^/  /' || echo "  (none)"

      # The structural negative control: if this is non-empty at ANY point,
      # decode is not going through Venus.
      echo "dev_video_opened=$(ls /dev/video* 2>/dev/null | tr '\n' ' ' || true)"

      echo "===VENUS-LAB-FIREFOX-END==="
    '';
  };

  documentation.enable = false;
  documentation.nixos.enable = false;
}
