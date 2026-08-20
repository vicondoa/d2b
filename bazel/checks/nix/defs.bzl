load("@bazel_skylib//rules:native_binary.bzl", "native_test")

_NIX_TAGS = [
    "exclusive",
    "no-remote-cache",
    "no-remote-exec",
    "no-sandbox",
]

_NIX_TOOL_LABELS = [
    "@nix//:bin/nix",
    "@python3//:bin/python3",
    "//:flake.nix",
    "//:tests/tools/peak-rss.py",
]

_NIX_SURFACE_TOOL_LABELS = [
    "@nix//:bin/nix",
    "@python3//:bin/python3",
    "//:tests/tools/peak-rss.py",
]

_NIX_SURFACE_TAGS = [
    "no-remote-exec",
    "no-sandbox",
]

_NIX_TEST_SCRIPT = """#!/bin/sh
set -eu

runfile() {
    key=$1
    if [ -n "${RUNFILES_DIR:-}" ] && [ -e "$RUNFILES_DIR/$key" ]; then
        printf '%s\\n' "$RUNFILES_DIR/$key"
        return 0
    fi
    if [ -n "${RUNFILES_MANIFEST_FILE:-}" ] && [ -r "$RUNFILES_MANIFEST_FILE" ]; then
        while IFS= read -r line || [ -n "$line" ]; do
            case "$line" in
                "$key "*)
                    printf '%s\\n' "${line#"$key "}"
                    return 0
                    ;;
            esac
        done <"$RUNFILES_MANIFEST_FILE"
    fi
    if [ -e "$key" ]; then
        printf '%s\\n' "$key"
        return 0
    fi
    printf 'Bazel runfile is missing: %s\\n' "$key" >&2
    return 1
}

NIX_BIN=$(runfile "__NIX_KEY__")
PYTHON_BIN=$(runfile "__PYTHON_KEY__")
PEAK_RSS=$(runfile "__PEAK_KEY__")
FLAKE_PATH=$(runfile "__FLAKE_KEY__")
ROOT=${FLAKE_PATH%/flake.nix}
if [ "$ROOT" = "$FLAKE_PATH" ]; then
    ROOT=.
fi
ROOT=$(CDPATH= cd -- "$ROOT" && pwd -P)
if [ -n "${D2B_REPO_ROOT:-}" ]; then
    ROOT=$D2B_REPO_ROOT
fi
cd "$ROOT"
export ROOT
export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"
if [ -n "${D2B_REPO_ROOT:-}" ]; then
    FLAKE_REF="git+file://$ROOT"
else
    FLAKE_REF="$ROOT"
fi
SYSTEM=$("$NIX_BIN" eval --raw --impure --expr builtins.currentSystem)
exec "$PYTHON_BIN" "$PEAK_RSS" \
    --lane "__LANE__" \
    --max-kib "__MAX_KIB__" \
    -- "$NIX_BIN" __COMMAND__
"""

def nix_native_test(name, command, lane, max_kib, data, timeout = "long"):
    runner = name + "_runner.sh"
    script = _NIX_TEST_SCRIPT
    script = script.replace("__NIX_KEY__", "$(rlocationpath @nix//:bin/nix)")
    script = script.replace("__PYTHON_KEY__", "$(rlocationpath @python3//:bin/python3)")
    script = script.replace("__PEAK_KEY__", "$(rlocationpath //:tests/tools/peak-rss.py)")
    script = script.replace("__FLAKE_KEY__", "$(rlocationpath //:flake.nix)")
    script = script.replace("__LANE__", lane)
    script = script.replace("__MAX_KIB__", str(max_kib))
    script = script.replace("__COMMAND__", command)
    make_vars = {
        "$(rlocationpath @nix//:bin/nix)": "__NIX_RUNFILE__",
        "$(rlocationpath @python3//:bin/python3)": "__PYTHON_RUNFILE__",
        "$(rlocationpath //:flake.nix)": "__FLAKE_RUNFILE__",
        "$(rlocationpath //:tests/tools/peak-rss.py)": "__PEAK_RUNFILE__",
    }
    for make_var, placeholder in make_vars.items():
        script = script.replace(make_var, placeholder)
    script = script.replace("$", "$$")
    for make_var, placeholder in make_vars.items():
        script = script.replace(placeholder, make_var)
    native.genrule(
        name = name + "_runner",
        outs = [runner],
        tools = _NIX_TOOL_LABELS,
        cmd = "\"$(execpath @python3//:bin/python3)\" -c 'import pathlib,sys; p=pathlib.Path(sys.argv[1]); p.write_text(sys.stdin.read()); p.chmod(0o755)' \"$(OUTS)\" <<'EOF'\n%s\nEOF" % script,
    )
    native_test(
        name = name,
        src = ":" + runner,
        data = data + [
            "//:flake.nix",
            "//:tests/tools/peak-rss.py",
            "@nix//:bin/nix",
            "@python3//:bin/python3",
        ],
        tags = _NIX_TAGS,
        timeout = timeout,
    )

_NIX_SURFACE_TEST_SCRIPT = """#!/bin/sh
set -eu

runfile() {
    key=$1
    if [ -n "${RUNFILES_DIR:-}" ] && [ -e "$RUNFILES_DIR/$key" ]; then
        printf '%s\\n' "$RUNFILES_DIR/$key"
        return 0
    fi
    if [ -n "${RUNFILES_MANIFEST_FILE:-}" ] && [ -r "$RUNFILES_MANIFEST_FILE" ]; then
        while IFS= read -r line || [ -n "$line" ]; do
            case "$line" in
                "$key "*)
                    printf '%s\\n' "${line#"$key "}"
                    return 0
                    ;;
            esac
        done <"$RUNFILES_MANIFEST_FILE"
    fi
    if [ -e "$key" ]; then
        printf '%s\\n' "$key"
        return 0
    fi
    printf 'Bazel runfile is missing: %s\\n' "$key" >&2
    return 1
}

copy_input() {
    source=$(runfile "$1")
    destination="$ISOLATED_ROOT/$2"
    mkdir -p "$(dirname "$destination")"
    cp -L -- "$source" "$destination"
}

NIX_BIN=$(runfile "__NIX_KEY__")
PYTHON_BIN=$(runfile "__PYTHON_KEY__")
PEAK_RSS=$(runfile "__PEAK_KEY__")
OUTPUT_ROOT=${TEST_UNDECLARED_OUTPUTS_DIR:-"$PWD/.nix-surface-output"}
ISOLATED_ROOT="$OUTPUT_ROOT/input-root"
rm -rf -- "$ISOLATED_ROOT"
mkdir -p "$ISOLATED_ROOT"

__COPY_INPUTS__

cat >"$ISOLATED_ROOT/surface-spec.json" <<'SURFACE_SPEC_EOF'
__SURFACE_SPEC_JSON__
SURFACE_SPEC_EOF
cat >"$ISOLATED_ROOT/surface-entry.nix" <<'SURFACE_ENTRY_EOF'
import ./tests/unit/nix/run-surface.nix {
  root = ./.;
  specPath = ./surface-spec.json;
}
SURFACE_ENTRY_EOF

unset D2B_REPO_ROOT
export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"
exec "$PYTHON_BIN" "$PEAK_RSS" \
    --lane "__LANE__" \
    --max-kib "__MAX_KIB__" \
    -- "$NIX_BIN" eval \
        --raw \
        --file "$ISOLATED_ROOT/surface-entry.nix"
"""

def _workspace_path(label):
    if not label.startswith("//"):
        fail("standalone Nix surface input must be a main-workspace file label: " + label)
    package_and_target = label[2:].split(":")
    if len(package_and_target) != 2:
        fail("standalone Nix surface input must use //package:path syntax: " + label)
    package = package_and_target[0]
    target = package_and_target[1]
    return package + "/" + target if package else target

def _surface_expression(surface, spec):
    return spec.get(
        "expression",
        "//tests/unit/nix:surfaces/" + surface + ".nix",
    )

def _source_label(path, spec):
    package = spec.get("package")
    if package and path.startswith(package + "/"):
        return "//" + package + ":" + path[len(package) + 1:]
    return "//:" + path

def _surface_runner_script(name, surface, spec, inputs):
    copy_lines = []
    make_vars = {
        "$(rlocationpath @nix//:bin/nix)": "__NIX_RUNFILE__",
        "$(rlocationpath @python3//:bin/python3)": "__PYTHON_RUNFILE__",
        "$(rlocationpath //:tests/tools/peak-rss.py)": "__PEAK_RUNFILE__",
    }
    for index, label in enumerate(inputs):
        make_var = "$(rlocationpath %s)" % label
        placeholder = "__SURFACE_INPUT_%d__" % index
        make_vars[make_var] = placeholder
        copy_lines.append(
            'copy_input "%s" "%s"' % (make_var, _workspace_path(label)),
        )

    script = _NIX_SURFACE_TEST_SCRIPT
    script = script.replace("__NIX_KEY__", "$(rlocationpath @nix//:bin/nix)")
    script = script.replace("__PYTHON_KEY__", "$(rlocationpath @python3//:bin/python3)")
    script = script.replace("__PEAK_KEY__", "$(rlocationpath //:tests/tools/peak-rss.py)")
    script = script.replace("__COPY_INPUTS__", "\n".join(copy_lines))
    script = script.replace("__LANE__", "nix-unit-" + surface)
    script = script.replace("__MAX_KIB__", str(10683720))
    script = script.replace("__SURFACE_SPEC_JSON__", json.encode({
        "modules": spec["modules"],
        "name": name,
        "surface": _workspace_path(_surface_expression(surface, spec)),
        "system": "x86_64-linux",
    }))
    for make_var, placeholder in make_vars.items():
        script = script.replace(make_var, placeholder)
    script = script.replace("$", "$$")
    for make_var, placeholder in make_vars.items():
        script = script.replace(placeholder, make_var)
    return script

def nix_surface_test(name, surface, spec):
    inputs = _surface_inputs(surface, spec)
    runner = name + "_runner.sh"
    script = _surface_runner_script(name, surface, spec, inputs)
    native.filegroup(
        name = name.replace("-", "_") + "_inputs",
        srcs = inputs,
    )
    native.genrule(
        name = name + "_runner",
        outs = [runner],
        tags = _NIX_SURFACE_TAGS,
        tools = _NIX_SURFACE_TOOL_LABELS + inputs,
        cmd = "\"$(execpath @python3//:bin/python3)\" -c 'import pathlib,sys; p=pathlib.Path(sys.argv[1]); p.write_text(sys.stdin.read()); p.chmod(0o755)' \"$(OUTS)\" <<'EOF'\n%s\nEOF" % script,
    )
    native_test(
        name = name,
        src = ":" + runner,
        data = inputs + _NIX_SURFACE_TOOL_LABELS,
        tags = _NIX_SURFACE_TAGS,
        timeout = "long",
    )

def _surface_inputs(surface, spec):
    modules = []
    for module in spec["modules"]:
        if module not in modules:
            modules.append(module)
    inputs = [
        "//:flake.lock",
        "//tests/unit/nix:eval-jobs.nix",
        "//tests/unit/nix:run-surface.nix",
        _surface_expression(surface, spec),
    ]
    if not spec.get("raw_cases", False):
        inputs += [
            "//:nix/test-support/eval-surface.nix",
            "//:nixos-modules/lib.nix",
            "//tests/unit/nix:default.nix",
            "//tests/unit/nix:helpers/eval.nix",
            "//tests/unit/nix:helpers/surface.nix",
        ]
    return inputs + [
        "//tests/unit/nix:cases/" + case + ".nix"
        for case in spec["cases"]
    ] + [
        "//tests/unit/nix:eval-cases/" + fixture + ".nix"
        for fixture in spec.get("fixtures", [])
    ] + [
        _source_label(module, spec)
        for module in modules
    ] + [
        _source_label(path, spec)
        for path in spec.get("files", [])
    ]

def nix_surface_suite(surfaces):
    surface_tests = []
    for surface, spec in surfaces.items():
        test_name = "nix-unit-" + surface
        nix_surface_test(
            name = test_name,
            surface = surface,
            spec = spec,
        )
        native.alias(
            name = "nix_unit_" + surface.replace("-", "_"),
            actual = ":" + test_name,
        )
        surface_tests.append(":" + test_name)
    native.test_suite(
        name = "nix_unit",
        tests = surface_tests + [
            ":nix-unit-provider-network-local",
            ":nix-unit-provider-device-usbip",
            ":nix-unit-provider-device-security-key",
            ":nix-unit-provider-device-tpm",
            ":nix-unit-provider-device-gpu",
            ":nix-unit-provider-volume-local",
            ":nix-unit-provider-activation-nixos",
            ":nix-unit-provider-audio-pipewire",
            ":nix-unit-provider-clipboard-wayland",
            ":nix-unit-provider-display-wayland",
            ":nix-unit-provider-notification-desktop",
        ],
    )
