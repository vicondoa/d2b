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
check="$8"
shift 8

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

# How this action reaches the store, decided once because it decides both the
# flags and whether realizing the closure escalates at all.
#
# A `local` store writes into the store directory itself, so it is only
# available where that directory is writable. A daemon that reports this user
# as trusted builds and adds the paths itself, which is the multi-user answer
# to the same question - and it is the one that must not be paired with a
# `local` store, because that store bypasses the daemon and would then need a
# write permission the user does not have. Only the third case, a store this
# process cannot write and a daemon that does not trust it, escalates. A
# Bazel action has no terminal, so an escalation that cannot proceed fails
# here - naming what is missing - rather than blocking on a password prompt
# that nothing will answer.
store_options=""
escalate=0
if [ -w "$store_dir" ]; then
  store_options="--option store local?store=$store_dir"
elif "$nix_bin" store ping --json 2>/dev/null | grep -q '"trusted":true'; then
  # The daemon is the store: no override, and no escalation. The probe reads
  # the machine output, which is the one on stdout: the human rendering of
  # `store ping` goes to stderr, so a probe that discards stderr would read
  # nothing at all and an escalated action would look untrusted.
  :
elif [ -n "$sudo_bin" ]; then
  store_options="--option store local?store=$store_dir"
  escalate=1
else
  fail "the nix store is not writable and the nix daemon does not trust this user, so the guest closure cannot be realized on this host"
fi

nix_run() {
  if [ "$escalate" -eq 1 ]; then
    "$sudo_bin" -E "$@"
  else
    "$@"
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
# The check's fixture is staged with everything else and reaches the
# evaluation as a path relative to this tree, which is how the guest finds
# its own `./lib.nix` and its node module again: a change to the fixture is a
# change to the image, not a number the image already recorded.
check_argument="[ ]"
if [ -n "$check" ]; then
  [ -f "$source/$check" ] || fail "the declared check fixture ($check) is not among the copied sources"
  check_argument="[ \\"$check\\" ]"
fi

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
expr="(builtins.getFlake \\"path:$root\\").guestImage.\\"$system\\" { rawBundle = \\"$bundle\\"; rawCloudHypervisorController = $controller_argument; extraModules = $check_argument; nodeShape = \\"$node_shape\\"; }"
echo "guest-image $label: realizing the guest from declared inputs" >&2
image="$(nix_run "$nix_bin" build \\
  $store_options \\
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
    check = ctx.attr.check

    # The check's fixture is a declared source like every other input, and it
    # reaches the evaluation as a path relative to the tree the action copies
    # rather than as a path into the execroot: an absolute path would make
    # `import` copy that one file into the store under a flat name, and its
    # own `./lib.nix` would then resolve outside any tree at all. Naming it
    # as a path rather than as a label is also what lets the fixtures stay
    # outside a Bazel package - a package boundary there would make the root
    # package's own reference to one of them an invalid label.
    if check:
        staged = [source.short_path for source in ctx.files.srcs]
        if check not in staged:
            fail("guest_image %s: the check fixture %s is not among srcs" % (ctx.label, check))
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
            check,
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
        # The check this guest belongs to, as a declared label on that check's
        # own fixture file. The guest's node, its machine size, its drive
        # layout and its assertions are all read out of that file during the
        # evaluation, so the lane carries no list of which check wants which
        # guest: the only place a check's guest is declared is the check.
        #
        # Left empty for a check whose assertions are the lane's own Rust: its
        # fixture is gone, so there is nothing to read a guest out of, and the
        # evaluation reads the check's node from the reusable node module's
        # table of ported checks instead - by the `node_shape` this target
        # declares, which is the check's own name.
        "check": attr.string(),
        # The name this guest reports on its console. With a `check` it is
        # the check's own name, which is what makes a launcher that booted
        # the wrong image say so in one line. Without one it names the shape
        # of the re-homed node the image evaluates: `daemon` for the
        # daemon/broker host shape, `writable-store` for the shape that
        # replaces the root drive and boots through a bootloader. It is a
        # declared attribute rather than a lane constant, so the guest a
        # target builds is part of the action's key rather than an ambient
        # fact.
        "node_shape": attr.string(
            default = "daemon",
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
export D2B_TEST_VM_HARNESS_CYCLES="{cycles}"
export D2B_TEST_VM_HARNESS_EMULATOR="$runfiles/__EMULATOR__"
export D2B_TEST_VM_HARNESS_IMAGE="$runfiles/__IMAGE__"
# A lane-scoped working directory that outlives each individual guest, and is
# this test's own rather than the sandboxed temporary directory the current
# Bazel release does not expose to a sandboxed action. The harness resolves a
# relative one against its own working directory.
export D2B_TEST_VM_HARNESS_WORK_ROOT="{work_root}"

exec "$runfiles/__HARNESS__" "$@"
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
    _write_runner(
        name = name,
        script = _LANE_RUNNER_SCRIPT.format(
            cycles = "2",
            work_root = "d2b-vm-lane-work/%s" % name,
        ),
        runfiles = {
            "__HARNESS__": harness,
            "__EMULATOR__": emulator,
            "__IMAGE__": image,
        },
        srcs = [emulator, harness, image],
        tags = _LANE_TAGS,
        timeout = timeout,
    )

def _write_runner(name, script, runfiles, srcs, tags, timeout):
    """Write a generated shell runner and register the test that runs it.

    The runner is written through a genrule rather than handed to the test
    rule's `env`, following `nix_native_test`: a runfile location is expanded
    only where a rule expands it, and the harness is a binary, not a shell
    script that could resolve its own runfiles. Each location becomes a
    placeholder first so the `$` escaping genrule's own expansion needs does
    not touch the shell script around it.
    """
    locations = list(runfiles.items())
    for index, (placeholder, _) in enumerate(locations):
        script = script.replace(placeholder, "@LANE_RUNFILE_%d@" % index)
    script = script.replace("$", "$$")
    for index, (_, label) in enumerate(locations):
        script = script.replace("@LANE_RUNFILE_%d@" % index, "$(rlocationpath %s)" % label)
    native.genrule(
        name = name + "_runner.sh",
        srcs = srcs,
        outs = [name + "_runner"],
        cmd = "\"$(execpath @python3//:bin/python3)\" -c 'import pathlib,sys; p=pathlib.Path(sys.argv[1]); p.write_text(sys.stdin.read()); p.chmod(0o755)' \"$(OUTS)\" <<'EOF'\n%s\nEOF" % script,
        tags = tags,
        tools = ["@python3//:bin/python3"],
    )
    native_test(
        name = name,
        # The genrule's output file, not the genrule target: `src` names a
        # file the test rule reads, and a label that resolves to a rule
        # rather than to an output leaves it with no file to run.
        src = ":" + name + "_runner",
        data = srcs,
        size = "large",
        tags = tags,
        timeout = timeout,
    )

_LANE_SUITE_RUNNER_SCRIPT = """\
#!/bin/sh
set -eu

# A test runs with its working directory inside the runfiles tree, not at its
# root, so the root is derived from this script's own location rather than
# assumed - a wrong guess here is a guest that never boots, with a path error
# instead of a boot error.
runfiles="$(CDPATH= cd -- "$(dirname -- "$0")/../../../.." && pwd -P)"

export D2B_TEST_VM_HARNESS_EMULATOR="$runfiles/__EMULATOR__"
export D2B_TEST_VM_HARNESS_IMAGES="$runfiles/__IMAGES__"
# A check that has not been ported yet is a Python script, and the
# interpreter it runs under is part of what the lane is. Left to itself the
# surface resolves whatever `python3` a developer's shell happens to find, so
# the interpreter is a declared runfile from the same pinned nix package set
# as the guest it drives.
export D2B_TEST_VM_HARNESS_PYTHON="$runfiles/__PYTHON__"
# A lane-scoped working directory that outlives each individual guest, and is
# this test's own rather than the sandboxed temporary directory the current
# Bazel release does not expose to a sandboxed action. The harness resolves a
# relative one against its own working directory.
export D2B_TEST_VM_HARNESS_WORK_ROOT="d2b-vm-lane-work/{name}"

# The harness is asked for the `lane` subcommand rather than being left to its
# own default. With no argument it runs the single-guest self-check, which is
# the other target's job and which asks for `D2B_TEST_VM_HARNESS_IMAGE` - a variable
# the lane has no reason to set, because the lane reads the whole image list
# instead. Anything a contributor passes on the command line reaches the
# lane's own selection, which reads `--check`.
exec "$runfiles/__HARNESS__" lane "$@"
"""

def lane_test(name, images, emulator, harness, python, timeout = "eternal"):
    """The lane: one guest per check's own configuration, pooled and run.

    The images are the graph outputs of one `guest_image` per check, so each
    check's guest is built from that check's own fixture. The pool the
    harness builds from them is sized from the distinct emulator invocations
    among them, not from a count chosen here: adding a check to the lane adds
    a check here, and the pool follows.

    The target's result is never cacheable. A guest's verdict depends on
    what the host did while it ran, so a second invocation re-runs every
    selected check rather than replaying what the first one concluded, and
    `no-cache` is what says that to the Bazel graph.

    The pool runs concurrently inside this one action, so the lane does not
    depend on Bazel scheduling several targets to overlap them, and live test
    output cannot serialize it either way. Each check reports under its own
    name in the JUnit document the action writes, which is what makes one
    failed check identifiable from the lane's own output.
    """
    listing = name + "_images.txt"
    native.genrule(
        name = name + "_images",
        srcs = images,
        outs = [listing],
        # A single `%s`, not `%%s`: this fragment is a plain string literal
        # that nothing `%`-formats, so the escape would reach the shell
        # doubled and `printf` would write a literal `%s` as the only line of
        # the listing. The `$(rlocationpath ...)` parts are expanded by Bazel
        # before the shell sees them, which is why the paths arrive intact.
        cmd = "printf '%s\\n' " + " ".join(["$(rlocationpath %s)" % image for image in images]) + " > $@",
        tags = _LANE_TAGS,
    )
    _write_runner(
        name = name,
        script = _LANE_SUITE_RUNNER_SCRIPT.format(name = name),
        runfiles = {
            "__HARNESS__": harness,
            "__EMULATOR__": emulator,
            "__IMAGES__": ":" + listing,
            "__PYTHON__": python,
        },
        # The images are the test's own data, not only the listing genrule's
        # inputs. A `$(rlocationpath)` line names a runfile of the *action*
        # that produced it, so without the images here the test's runfiles
        # tree carries the listing and nothing it points at, and the lane
        # fails on the first manifest it tries to read.
        srcs = [
            ":" + listing,
        ] + images + [
            emulator,
            harness,
            python,
        ],
        tags = _LANE_TAGS + ["no-cache"],
        timeout = timeout,
    )
