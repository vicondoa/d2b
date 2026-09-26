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

load("@bazel_skylib//rules:native_binary.bzl", "native_test")

_GUEST_IMAGE_COMMAND = """\
set -eu
label="%s"
node_shape="%s"

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
expr="(builtins.getFlake \\"path:$root\\").guestImage.\\"$system\\" { rawBundle = \\"$bundle\\"; rawCloudHypervisorController = $controller_argument; nodeShape = \\"$node_shape\\"; }"
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
        command = _GUEST_IMAGE_COMMAND % (
            str(ctx.label),
            ctx.attr.node_shape,
        ),
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
        # Which of the re-homed node's two guest shapes this image evaluates:
        # `daemon` for the daemon/broker host checks, `writable-store` for
        # the checks that boot a nested guest, which replaces the root drive
        # and boots through a bootloader. It is a declared attribute rather
        # than a lane constant, so the shape a guest is built from is part of
        # the action's key rather than an ambient fact.
        "node_shape": attr.string(
            default = "daemon",
            values = ["daemon", "writable-store"],
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



_LANE_RUNNER_SCRIPT = """\
#!/bin/sh
set -eu

# A test runs with its working directory inside the runfiles tree, not at its
# root, so the root is derived from this script's own location rather than
# assumed - a wrong guess here is a guest that never boots, with a path error
# instead of a boot error.
runfiles="$(CDPATH= cd -- "$(dirname -- "$0")/../../../.." && pwd -P)"

export D2B_VM_HARNESS_CYCLES="{cycles}"
export D2B_VM_HARNESS_EMULATOR="$runfiles/{emulator}"
export D2B_VM_HARNESS_IMAGE="$runfiles/{image}"
# A lane-scoped working directory that outlives each individual guest, and is
# this test's own rather than the sandboxed temporary directory the current
# Bazel release does not expose to a sandboxed action. The harness resolves a
# relative one against its own working directory.
export D2B_VM_HARNESS_WORK_ROOT="{work_root}"

exec "$runfiles/{harness}" "$@"
"""

_LANE_TAGS = [
    "exclusive",
    "local",
    "no-remote-cache",
    "no-remote-exec",
    "no-sandbox",
]

def guest_boot_test(name, image, emulator, harness, timeout = "eternal"):
    """Boot one guest shape, wait for its activation contract, and tear it down.

    The guest is booted by the lane's own harness, against the image the
    guest-image action produced from the re-homed guest node. The emulator
    arrives as a declared runfile from the pinned nix package set, so the
    emulator and the guest closure are at one nixpkgs revision.

    The image, the emulator, and the lane's working directory are handed to
    the harness through a generated runner rather than through the test
    rule's `env`: a runfile location is only expanded where a rule expands
    it, and the harness is a binary, not a shell script that could resolve
    its own runfiles.

    The target runs unsandboxed and local: a guest opens `/dev/kvm`, may run
    a nested guest, and owns a working directory that outlives an individual
    check, none of which a sandboxed action can provide. The tags keep it off
    remote execution and out of any aggregate that would replay a guest's
    verdict.
    """
    runner = name + "_runner.sh"
    script = _LANE_RUNNER_SCRIPT.format(
        cycles = "2",
        emulator = "$(rlocationpath %s)" % emulator,
        harness = "$(rlocationpath %s)" % harness,
        image = "$(rlocationpath %s)" % image,
        work_root = "d2b-vm-lane-work/%s" % name,
    )

    # The runner is written through a genrule rather than handed to the test
    # rule's `env`, following `nix_native_test`: a runfile location is
    # expanded only where a rule expands it, and a binary cannot resolve its
    # own runfiles. Each location becomes a placeholder first so the `$`
    # escaping genrule's own expansion needs does not touch the shell script
    # around it.
    make_vars = {
        "$(rlocationpath %s)" % harness: "__HARNESS__",
        "$(rlocationpath %s)" % emulator: "__EMULATOR__",
        "$(rlocationpath %s)" % image: "__IMAGE__",
    }
    for make_var, placeholder in make_vars.items():
        script = script.replace(make_var, placeholder)
    script = script.replace("$", "$$")
    for make_var, placeholder in make_vars.items():
        script = script.replace(placeholder, make_var)

    native.genrule(
        name = runner,
        srcs = [
            emulator,
            harness,
            image,
        ],
        outs = [name + "_runner"],
        cmd = "\"$(execpath @python3//:bin/python3)\" -c 'import pathlib,sys; p=pathlib.Path(sys.argv[1]); p.write_text(sys.stdin.read()); p.chmod(0o755)' \"$(OUTS)\" <<'EOF'\n%s\nEOF" % script,
        tags = _LANE_TAGS,
        tools = ["@python3//:bin/python3"],
    )
    native_test(
        name = name,
        src = ":" + name + "_runner",
        data = [
            ":" + name + "_runner",
            harness,
            image,
            emulator,
        ],
        size = "large",
        tags = _LANE_TAGS,
        timeout = timeout,
    )
