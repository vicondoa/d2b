#!/usr/bin/env bash
# host-caps.sh - capture the host's Vulkan Video capability report.
#
# Answers the W0 question: does the host NVIDIA driver actually expose the
# Vulkan Video pieces we intend to forward through Venus? If it does not, the
# entire prototype is blocked below us and there is nothing to build.
#
# Runs in two modes:
#
#   (default)  probe on the plain host
#   --in-sandbox
#              re-exec under the SAME bubblewrap bind set the crosvm GPU sidecar
#              will use, then probe
#
# Running both is the point. If /nix/store or /run/opengl-driver are missing from
# the sandbox, the NVIDIA ICD's library_path dangles and the sidecar enumerates
# NO NVIDIA Vulkan Video at all -- while the plain-host probe still passes. That
# divergence is invisible unless you measure inside the namespace, and it would
# otherwise look like a Venus bug for days.
set -euo pipefail

STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab"
OUT_DIR="${VENUS_LAB_OUT:-$STATE_DIR/evidence}"

# The extensions Firefox's PhysicalDeviceHasVulkanVideoDecodeStack() hard-gates
# on, plus the H.264 decode extension we are actually forwarding.
REQUIRED_EXTS=(
  VK_KHR_video_queue
  VK_KHR_video_decode_queue
  VK_KHR_video_decode_h264
)

# Bind set must stay in sync with the sandbox contract in AGENTS.md rule 6.
bwrap_args() {
  local args=(
    --ro-bind /nix/store /nix/store
    --ro-bind /run/opengl-driver /run/opengl-driver
    --ro-bind /etc /etc
    --proc /proc
    --tmpfs /tmp
    --dev /dev
  )
  # Probe-harness convenience only: this makes coreutils/grep/sed/awk
  # resolvable inside the namespace. It is a symlink farm into /nix/store,
  # which is already bound read-only, so it grants no new access. The real
  # crosvm GPU sidecar is a single binary and does NOT need this bind.
  [ -d /run/current-system/sw ] &&
    args+=(--ro-bind /run/current-system/sw /run/current-system/sw)
  # The broad /etc bind is needed for the Vulkan loader and NSS config, but it
  # would also expose /etc/d2b to the sandbox, which AGENTS.md rule 3 forbids.
  # Mask it with an empty tmpfs; the probe asserts it is invisible.
  [ -e /etc/d2b ] && args+=(--tmpfs /etc/d2b)
  # /sys is required: NVIDIA userspace enumerates the GPU through sysfs, and
  # without it vkCreateInstance segfaults inside the namespace even though the
  # ICD and device nodes are present.
  [ -d /sys ] && args+=(--ro-bind /sys /sys)
  # Devices are bound individually rather than exposing all of /dev.
  # nvidia-modeset and nvidia-uvm-tools are included because the NVIDIA
  # userspace stack probes them during instance creation.
  local d
  for d in /dev/dri/renderD128 /dev/nvidia0 /dev/nvidiactl /dev/nvidia-modeset \
           /dev/nvidia-uvm /dev/nvidia-uvm-tools /dev/udmabuf; do
    [ -e "$d" ] && args+=(--dev-bind "$d" "$d")
  done
  printf '%s\n' "${args[@]}"
}

have() { command -v "$1" >/dev/null 2>&1; }

# Absolute path to this script, so the bwrap re-exec can bind and find it.
# Avoid depending on readlink here: inside the sandbox, coreutils may not be
# on PATH yet, and this line runs on both sides of the re-exec.
case "$0" in
  /*) SELF="$0" ;;
  *)  SELF="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")" ;;
esac

# vulkaninfo is not necessarily on PATH; pull it from the pinned nixpkgs the same
# way the rest of the repo's tooling does.
vulkaninfo_cmd() {
  if have vulkaninfo; then
    echo "vulkaninfo"
  elif have nix; then
    echo "nix shell --quiet nixpkgs#vulkan-tools --command vulkaninfo"
  else
    echo "error: neither vulkaninfo nor nix is available" >&2
    exit 1
  fi
}

probe() {
  local label="$1" report="$2"
  local vi; vi=$(vulkaninfo_cmd)

  mkdir -p "$OUT_DIR"
  if ! $vi > "$report" 2>&1; then
    echo "FAIL [$label]: vulkaninfo failed; see $report" >&2
    return 1
  fi

  echo "=== host Vulkan Video capability report [$label] ==="
  echo "full report: $report"

  # Attribute capabilities to a SPECIFIC device. A whole-file grep is a
  # false-pass trap: this host also exposes llvmpipe (software), and on a
  # misconfigured sandbox the NVIDIA ICD can vanish while some other device
  # still matches the strings elsewhere in the dump.
  local nvidia_start nvidia_end
  nvidia_start=$(awk '/^GPU[0-9]+:/{g=NR} /deviceName.*NVIDIA/{print g; exit}' "$report")
  if [ -z "$nvidia_start" ]; then
    echo "FAIL [$label]: no NVIDIA device found in the Vulkan report." >&2
    echo "  devices seen:" >&2
    grep -E "deviceName" "$report" | sed 's/^/    /' | sort -u >&2
    if [ "$label" = "sandbox" ]; then
      echo "  HINT: the sandbox is probably missing /nix/store or" >&2
      echo "  /run/opengl-driver, so the NVIDIA ICD's library_path dangles." >&2
    fi
    return 1
  fi
  # The NVIDIA block ends where the next GPU block begins (or at EOF).
  nvidia_end=$(awk -v s="$nvidia_start" 'NR>s && /^GPU[0-9]+:/{print NR; exit}' "$report")
  [ -n "$nvidia_end" ] || nvidia_end=$(wc -l < "$report")

  echo "--- NVIDIA device block: lines ${nvidia_start}-${nvidia_end} ---"
  # Extract the device block ONCE. Do not pipe into `grep -q` under
  # `set -o pipefail`: grep -q exits on first match, SIGPIPEs the upstream
  # sed, and pipefail then reports the whole pipeline as failed -- which
  # silently turns every PRESENT into MISSING.
  local nvidia_block
  nvidia_block=$(sed -n "${nvidia_start},${nvidia_end}p" "$report")

  printf '%s\n' "$nvidia_block" \
    | grep -E "deviceName|driverName|driverInfo" | sed 's/^/  /' | head -4

  local missing=0 ext
  for ext in "${REQUIRED_EXTS[@]}" QUEUE_VIDEO_DECODE_BIT_KHR; do
    if printf '%s\n' "$nvidia_block" | grep -c -- "$ext" >/dev/null; then
      printf '  %-38s PRESENT (on NVIDIA device)\n' "$ext"
    else
      printf '  %-38s MISSING\n' "$ext"
      missing=$((missing + 1))
    fi
  done

  # A video-capable queue family is necessary but NOT sufficient. FFmpeg needs a
  # queue family whose VkQueueFamilyVideoPropertiesKHR.videoCodecOperations
  # actually includes H.264 decode; without it a device can advertise the
  # extensions and expose a video queue while still having no usable H.264
  # queue, and session creation fails later for reasons that look unrelated.
  if printf '%s\n' "$nvidia_block" \
       | grep -c "VIDEO_CODEC_OPERATION_DECODE_H264" >/dev/null; then
    printf '  %-38s PRESENT (on NVIDIA device)\n' "videoCodecOperations H264"
  else
    printf '  %-38s MISSING\n' "videoCodecOperations H264"
    missing=$((missing + 1))
  fi

  if [ "$missing" -gt 0 ]; then
    echo "RESULT [$label]: $missing required capability/ies MISSING on the NVIDIA device" >&2
    return 1
  fi

  # Negative proof: the lab isolation contract says the sidecar must never see
  # d2b control-plane state. Assert it rather than assuming the tmpfs mask
  # worked.
  if [ "$label" = "sandbox" ]; then
    local d2b_visible
    d2b_visible=$(ls -A /etc/d2b 2>/dev/null | head -1 || true)
    if [ -n "$d2b_visible" ]; then
      echo "  /etc/d2b: VISIBLE -- isolation contract violated" >&2
      return 1
    fi
    echo "  /etc/d2b: masked (isolation contract holds)"
  fi

  echo "RESULT [$label]: all required Vulkan Video capabilities present on the NVIDIA device"
}

case "${1:-}" in
  --in-sandbox)
    if ! have bwrap; then
      echo "error: bwrap not found; cannot run the in-sandbox probe" >&2
      exit 1
    fi
    mkdir -p "$OUT_DIR"
    mapfile -t BW < <(bwrap_args)
    # Bind this script (the repo itself is deliberately NOT in the bind set)
    # and the evidence dir, which must be writable for the report.
    BW+=(--ro-bind "$SELF" "$SELF" --bind "$OUT_DIR" "$OUT_DIR")
    # Invoke bash by absolute path: /usr/bin/env is not in the bind set, so
    # relying on the `#!/usr/bin/env bash` shebang fails with a confusing
    # "No such file or directory" that looks like the script is missing.
    bash_abs=$(command -v bash)
    exec bwrap "${BW[@]}" -- "$bash_abs" "$SELF" --probe-only sandbox
    ;;
  --probe-only)
    probe "${2:-sandbox}" "$OUT_DIR/host-caps-${2:-sandbox}.txt"
    ;;
  ""|--host)
    probe host "$OUT_DIR/host-caps-host.txt"
    ;;
  -h|--help)
    sed -n '2,20p' "$0"
    ;;
  *)
    echo "usage: ${0##*/} [--host|--in-sandbox]" >&2
    exit 2
    ;;
esac
