def _nix_fixture_impl(ctx):
    output = ctx.actions.declare_directory(ctx.label.name)
    inputs = depset(
        ctx.files.srcs + [
            ctx.file.flake,
            ctx.file.materializer,
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
        ] + [source.path for source in ctx.files.srcs],
        command = """\
set -eu
export PATH=/run/current-system/sw/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:$PATH
nix_bin="$1"
python_bin="$2"
flake="$3"
materializer="$4"
out="$5"
variant="$6"
source="$out.source"
rm -rf "$source"
mkdir -p "$source"
for input in "${@:7}"; do
  destination="$source/$input"
  mkdir -p "$(dirname "$destination")"
  cp -L "$input" "$destination"
done
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
