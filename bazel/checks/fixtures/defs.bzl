def _nix_fixture_impl(ctx):
    output = ctx.actions.declare_directory(ctx.label.name)
    source_manifest = ctx.actions.declare_file(ctx.label.name + ".sources")
    ctx.actions.write(
        output = source_manifest,
        content = "\n".join([source.path for source in ctx.files.srcs]) + "\n",
    )
    inputs = depset(ctx.files.srcs + [ctx.file.flake, source_manifest])
    ctx.actions.run_shell(
        inputs = inputs,
        tools = [ctx.executable.nix] + ctx.files.host_tools,
        outputs = [output],
        arguments = [
            ctx.executable.nix.path,
            ctx.file.flake.path,
            output.path,
            ctx.attr.variant,
            source_manifest.path,
        ] + [tool.path for tool in ctx.files.host_tools],
        command = """\
set -eu
export PATH=/run/current-system/sw/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:$PATH
nix_bin="$1"
flake="$2"
out="$3"
variant="$4"
source_manifest="$5"
shift 5
source="$out.source"
rm -rf "$source"
mkdir -p "$source"
while IFS= read -r input; do
  [ -n "$input" ] || continue
  destination="$source/$input"
  mkdir -p "$(dirname "$destination")"
  cp -L "$input" "$destination"
done < "$source_manifest"
root="$(CDPATH= cd -- "$source" && pwd -P)"
tool_bundle="$root/.fixture-tools"
mkdir -p "$tool_bundle"
for tool in "$@"; do
  cp -L "$tool" "$tool_bundle/$(basename "$tool")"
done
mkdir -p "$out"
export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"
export D2B_FIXTURE_RESOURCE_COMPILER="$tool_bundle/d2b-resource-compiler"

case "$variant" in
  minimal)
    system="$("$nix_bin" eval --raw --impure --expr builtins.currentSystem)"
    store_path="$("$nix_bin" build --impure --no-write-lock-file --no-link --print-out-paths \
      "path:$root#checks.${system}.fixture-smoke")"
    cp -R "$store_path"/. "$out"/
    ;;
  *)
    printf 'unknown fixture variant: %s\\n' "$variant" >&2
    exit 2
    ;;
esac
rm -rf "$source"
""",
    )
    return [DefaultInfo(files = depset([output]))]

nix_fixture = rule(
    implementation = _nix_fixture_impl,
    attrs = {
        "flake": attr.label(allow_single_file = True),
        "host_tools": attr.label_list(
            allow_files = True,
            cfg = "exec",
        ),
        "nix": attr.label(
            allow_single_file = True,
            cfg = "exec",
            executable = True,
        ),
        "srcs": attr.label_list(allow_files = True),
        "variant": attr.string(mandatory = True),
    },
)
