def _nix_fixture_impl(ctx):
    output = ctx.actions.declare_directory(ctx.label.name)
    source_manifest = ctx.actions.declare_file(ctx.label.name + ".sources")
    ctx.actions.write(
        output = source_manifest,
        content = "\n".join([source.path for source in ctx.files.srcs]) + "\n",
    )
    inputs = depset(
        ctx.files.srcs + [
            ctx.file.flake,
            ctx.file.materializer,
            source_manifest,
        ],
    )
    ctx.actions.run_shell(
        inputs = inputs,
        tools = [
            ctx.executable.nix,
            ctx.executable.python3,
        ],
        outputs = [output],
        arguments = [
            ctx.executable.nix.path,
            ctx.executable.python3.path,
            ctx.file.flake.path,
            ctx.file.materializer.path,
            output.path,
            ctx.attr.variant,
            source_manifest.path,
        ],
        command = """\
set -eu
export PATH=/run/current-system/sw/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:$PATH
nix_bin="$1"
python_bin="$2"
flake="$3"
materializer="$4"
out="$5"
variant="$6"
source_manifest="$7"
source="$out.source"
rm -rf "$source"
mkdir -p "$source"
while IFS= read -r input; do
  [ -n "$input" ] || continue
  destination="$source/$input"
  mkdir -p "$(dirname "$destination")"
  cp -L "$input" "$destination"
done < "$source_manifest"
root="$source"
mkdir -p "$out"
export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"

case "$variant" in
  minimal)
    system="$("$nix_bin" eval --raw --impure --expr builtins.currentSystem)"
    store_path="$("$nix_bin" build --no-write-lock-file --no-link --print-out-paths \
      "path:$root#checks.${system}.fixture-smoke")"
    cp -R "$store_path"/. "$out"/
    ;;
  full)
    system="$("$nix_bin" eval --raw --impure --expr builtins.currentSystem)"
    [ "$system" = x86_64-linux ] || exit 0
    json="$out/eval-fixture.json"
    "$nix_bin" eval --quiet --no-write-lock-file --no-warn-dirty --json --apply \
      "fixtureFor: (fixtureFor \\"$system\\").full" \
      "path:$root#lib.evalFixture" > "$json"
    "$python_bin" "$materializer" "$json" "$out"
    rm -f "$json"
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
        "materializer": attr.label(allow_single_file = True),
        "nix": attr.label(
            allow_single_file = True,
            cfg = "exec",
            executable = True,
        ),
        "python3": attr.label(
            allow_single_file = True,
            cfg = "exec",
            executable = True,
        ),
        "srcs": attr.label_list(allow_files = True),
        "variant": attr.string(mandatory = True),
    },
)
