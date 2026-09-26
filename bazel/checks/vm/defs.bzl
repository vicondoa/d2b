"""Guest image for the Bazel-owned host-integration lane.

The lane builds its guest as a graph output rather than as a side effect of
a shell recipe: the flake and its lock, the guest module sources, and the
d2b host binaries all arrive as declared label inputs, so the image is
keyed on the source and rebuilds when any of them changes.

Modeled on `bazel/checks/fixtures/defs.bzl`, the repository's one existing
cacheable nix build action.

The action runs unsandboxed and local. Nix cannot build inside a Bazel
sandbox - its own sandbox needs root, and the fallback degrades silently
rather than failing - so hermeticity comes from the action's own nix
configuration: the store it builds into, the substituters it may fetch
from, and the build-users setting are all set here, and the flake arrives
as a copied input rather than as a working-tree reference.
"""

_GUEST_IMAGE_COMMAND = """\
set -eu
label="%s"

# Resolved before the fixed PATH is installed: on NixOS the setuid sudo is
# the wrapper, and the profile entry is not.
sudo_bin=""
for candidate in /run/wrappers/bin/sudo "$(command -v sudo 2>/dev/null || true)"; do
  if [ -n "$candidate" ] && [ -x "$candidate" ]; then
    sudo_bin="$candidate"
    break
  fi
done

# Fixed PATH, a scratch HOME, and an explicit feature set: none of the
# action's nix configuration may come from the developer's shell.
export PATH=/run/current-system/sw/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
export NIX_CONFIG="experimental-features = nix-command flakes"

nix_bin="$1"
flake="$2"
lock="$3"
out="$4"
source_manifest="$5"
substituters="$6"
controller="$7"
shift 7

# Bazel hands over execroot-relative paths. Nix resolves its configuration
# against the working directory, so the output tree is anchored here.
case "$out" in
  /*) ;;
  *) out="$(pwd)/$out" ;;
esac

store_dir=/nix/store
staging="$out.staging"
# The staged sources and binaries are scratch, on the failure path as much
# as on the success path.
trap 'rm -rf "$staging"' EXIT
fail() {
  echo "guest-image $label: $1" >&2
  exit 1
}
nix_run() {
  if [ -w "$store_dir" ]; then
    "$@"
  elif [ -n "$sudo_bin" ]; then
    "$sudo_bin" -E "$@"
  else
    fail "the nix store is not writable and no sudo is available, so the guest closure cannot be realized on this host"
  fi
}

rm -rf "$staging"
mkdir -p "$staging/home"
export HOME="$staging/home"
export XDG_CONFIG_HOME="$staging/home/config"
export XDG_CACHE_HOME="$staging/home/cache"

# The flake arrives as declared label inputs, so the action key reflects
# the source rather than a working-tree reference.
source="$staging/source"
mkdir -p "$source"
while IFS= read -r input; do
  [ -n "$input" ] || continue
  destination="$source/$input"
  mkdir -p "$(dirname "$destination")"
  cp -L "$input" "$destination"
done < "$source_manifest"
root="$(CDPATH= cd -- "$source" && pwd -P)"
[ -f "$source/$flake" ] || fail "the declared flake ($flake) is not among the copied sources"
[ -f "$source/$lock" ] || fail "the declared flake lock ($lock) is not among the copied sources"

# The d2b host binaries, staged by output name into the bundle the guest
# closure reads. The guest-side package refuses any other inventory, so a
# renamed or missing binary fails the build rather than producing a guest
# with a tool missing from it.
bundle="$staging/bundle"
mkdir -p "$bundle"
for tool in "$@"; do
  install -m 755 "$tool" "$bundle/$(basename "$tool")"
done
controller_bundle=""
if [ -n "$controller" ]; then
  controller_bundle="$staging/controller"
  mkdir -p "$controller_bundle"
  install -m 755 "$controller" "$controller_bundle/$(basename "$controller")"
  controller_argument="\\"$controller_bundle\\""
else
  controller_argument="null"
fi

# Preflight the declared substituters before building: an unreachable cache
# fails the action here rather than silently yielding an image from a
# partial closure.
reachable=0
for cache in $substituters; do
  if "$nix_bin" store info --store "$cache" >/dev/null 2>&1; then
    reachable=1
  else
    echo "guest-image $label: substituter $cache is unreachable" >&2
  fi
done
[ "$reachable" -eq 1 ] || fail "none of the declared substituters ($substituters) is reachable"

system="$("$nix_bin" eval --raw --impure --expr builtins.currentSystem)" || fail "could not read the nix system"
expr="(builtins.getFlake \\"path:$root\\").guestImage.\\"$system\\" { rawBundle = \\"$bundle\\"; rawCloudHypervisorController = $controller_argument; }"
echo "guest-image $label: realizing the guest from declared inputs" >&2
image="$(nix_run "$nix_bin" build \\
  --option store "local?store=$store_dir" \\
  --option substituters "$substituters" \\
  --option build-users-group "" \\
  --option sandbox true \\
  --option sandbox-fallback false \\
  --impure \\
  --no-write-lock-file \\
  --no-link \\
  --print-out-paths \\
  --expr "$expr")" || fail "the guest evaluation or build failed; see the nix output above"
[ -n "$image" ] && [ -d "$image" ] || fail "nix reported no guest image output ($image)"

rm -rf "$staging"
mkdir -p "$out"
cp -a "$image/." "$out/"
[ -f "$out/manifest.json" ] || fail "the realized guest image carries no manifest"
"""

def _guest_image_impl(ctx):
    output = ctx.actions.declare_directory(ctx.label.name)
    source_manifest = ctx.actions.declare_file(ctx.label.name + ".sources")
    ctx.actions.write(
        output = source_manifest,
        content = "\n".join([source.path for source in ctx.files.srcs]) + "\n",
    )
    controller = ctx.file.cloud_hypervisor_controller
    ctx.actions.run_shell(
        inputs = depset(
            ctx.files.srcs + ctx.files.host_tools + ctx.files.cloud_hypervisor_controller +
            [ctx.file.flake, ctx.file.flake_lock, source_manifest],
        ),
        tools = [ctx.executable.nix] + ctx.files.host_tools + ctx.files.cloud_hypervisor_controller,
        outputs = [output],
        arguments = [
            ctx.executable.nix.path,
            ctx.file.flake.short_path,
            ctx.file.flake_lock.short_path,
            output.path,
            source_manifest.path,
            ctx.attr.substituters,
            controller.path if controller else "",
        ] + [tool.path for tool in ctx.files.host_tools],
        command = _GUEST_IMAGE_COMMAND % {"label": str(ctx.label)},
        mnemonic = "GuestImage",
        progress_message = "Building d2b guest image %s" % ctx.label.name,
    )
    return [DefaultInfo(files = depset([output]))]

guest_image = rule(
    implementation = _guest_image_impl,
    doc = "Realize the host-integration guest image from declared inputs.",
    attrs = {
        "cloud_hypervisor_controller": attr.label(
            allow_single_file = True,
        ),
        "flake": attr.label(
            allow_single_file = True,
            mandatory = True,
        ),
        "flake_lock": attr.label(
            allow_single_file = True,
            mandatory = True,
        ),
        # The guest's own binaries, taken in the target configuration: the
        # guest runs the binaries this build produces, not a second copy
        # built for the action's own execution platform.
        "host_tools": attr.label_list(
            allow_files = True,
            mandatory = True,
        ),
        "nix": attr.label(
            allow_single_file = True,
            cfg = "exec",
            executable = True,
            mandatory = True,
        ),
        "srcs": attr.label_list(allow_files = True, mandatory = True),
        # The caches this action may fetch the guest closure from. They are
        # declared rather than read from the host so an unreachable cache
        # fails the action, and so a developer's nix configuration cannot
        # change what the guest is built from.
        "substituters": attr.string(
            default = "https://cache.nixos.org/",
        ),
    },
)
