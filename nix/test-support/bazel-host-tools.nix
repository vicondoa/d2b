{ pkgs, rawBundle }:

let
  inherit (pkgs) lib;

  inventory = [
    "d2b"
    "d2bd"
    "d2b-priv-broker"
    "d2b-activation-helper"
    "d2b-host-activation-helper"
    "d2b-cutover-runner"
    "d2b-unsafe-local-helper"
    "d2b-resource-compiler"
    "d2b-wayland-proxy"
  ];
  inventoryShell = lib.concatStringsSep " " (map (name: "'${name}'") inventory);
  overrideKeys = [
    "d2b"
    "d2bd"
    "broker"
    "activationHelper"
    "hostActivationHelper"
    "cutoverRunner"
    "unsafeLocalHelper"
    "resourceCompiler"
    "waylandProxy"
  ];

  package = pkgs.stdenv.mkDerivation {
    pname = "d2b-bazel-host-tools";
    version = "0.0.0";
    src = rawBundle;
    strictDeps = true;
    dontUnpack = true;
    dontConfigure = true;
    dontBuild = true;

    nativeBuildInputs = [
      pkgs.autoPatchelfHook
      pkgs.binutils
      pkgs.coreutils
      pkgs.findutils
      pkgs.gnugrep
      pkgs.patchelf
    ];
    buildInputs = [
      pkgs.glibc
      pkgs.stdenv.cc.cc.lib
    ];

    installPhase = ''
      runHook preInstall

      expected="$(printf '%s\n' ${inventoryShell} | sort)"
      actual="$(find -P "$src" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)"
      if [ "$actual" != "$expected" ]; then
        echo "d2b-bazel-host-tools: raw bundle inventory mismatch" >&2
        echo "expected:" >&2
        printf '%s\n' "$expected" >&2
        echo "actual:" >&2
        printf '%s\n' "$actual" >&2
        exit 1
      fi

      for name in ${inventoryShell}; do
        source="$src/$name"
        if [ ! -f "$source" ] || [ -L "$source" ] || [ ! -x "$source" ]; then
          echo "d2b-bazel-host-tools: $name must be a regular executable file" >&2
          exit 1
        fi

        install -Dm755 "$source" "$out/bin/$name"
      done

      topEntries="$(find -P "$out" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)"
      if [ "$topEntries" != "bin" ]; then
        echo "d2b-bazel-host-tools: unexpected installed top-level entries" >&2
        printf '%s\n' "$topEntries" >&2
        exit 1
      fi
      installed="$(find -P "$out/bin" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)"
      if [ "$installed" != "$expected" ]; then
        echo "d2b-bazel-host-tools: installed inventory mismatch" >&2
        exit 1
      fi

      runHook postInstall
    '';

    doInstallCheck = true;
    installCheckPhase = ''
      runHook preInstallCheck

      readelf=${pkgs.binutils}/bin/readelf
      patchelf=${pkgs.patchelf}/bin/patchelf

      topEntries="$(find -P "$out" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)"
      if [ "$topEntries" != "bin" ]; then
        echo "d2b-bazel-host-tools: unexpected installed top-level entries after fixup" >&2
        printf '%s\n' "$topEntries" >&2
        exit 1
      fi
      expected="$(printf '%s\n' ${inventoryShell} | sort)"
      installed="$(find -P "$out/bin" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)"
      if [ "$installed" != "$expected" ]; then
        echo "d2b-bazel-host-tools: installed inventory changed during fixup" >&2
        exit 1
      fi

      checkElf() {
        name="$1"
        bin="$2"
        test -f "$bin"
        test ! -L "$bin"

        header="$("$readelf" -h "$bin" 2>/dev/null)" || {
          echo "d2b-bazel-host-tools: $name is not an ELF binary after fixup" >&2
          exit 1
        }
        if ! printf '%s\n' "$header" | grep -Eq 'Class:[[:space:]]+ELF64'; then
          echo "d2b-bazel-host-tools: $name is not ELF64 after fixup" >&2
          exit 1
        fi
        if ! printf '%s\n' "$header" | grep -Eq 'Data:[[:space:]]+2'; then
          echo "d2b-bazel-host-tools: $name is not little-endian after fixup" >&2
          exit 1
        fi
        if ! printf '%s\n' "$header" | grep -Eq 'Machine:[[:space:]]+(Advanced Micro Devices X86-64|x86-64)'; then
          echo "d2b-bazel-host-tools: $name is not x86_64 after fixup" >&2
          exit 1
        fi

        interpreter="$("$patchelf" --print-interpreter "$bin" 2>/dev/null)" || {
          echo "d2b-bazel-host-tools: $name has no ELF interpreter" >&2
          exit 1
        }
        case "$interpreter" in
          /nix/store/*/lib/ld-linux-x86-64.so.2|/nix/store/*/lib64/ld-linux-x86-64.so.2)
            ;;
          *)
            echo "d2b-bazel-host-tools: $name does not use a Nix-store x86_64 loader" >&2
            echo "$interpreter" >&2
            exit 1
            ;;
        esac
        test -x "$interpreter"

        runtime="$("$interpreter" --list "$bin" 2>&1)" || {
          echo "d2b-bazel-host-tools: failed to resolve runtime libraries for $name" >&2
          printf '%s\n' "$runtime" >&2
          exit 1
        }
        if printf '%s\n' "$runtime" | grep -qi 'not found'; then
          echo "d2b-bazel-host-tools: unresolved runtime library for $name" >&2
          printf '%s\n' "$runtime" >&2
          exit 1
        fi
        while IFS= read -r line; do
          case "$line" in
            *"=>"*)
              resolved="''${line#*=> }"
              resolved="''${resolved%% *}"
              case "$resolved" in
                /nix/store/*)
                  ;;
                *)
                  echo "d2b-bazel-host-tools: $name resolves a library outside /nix/store" >&2
                  echo "$resolved" >&2
                  exit 1
                  ;;
              esac
              ;;
          esac
        done <<< "$runtime"
      }

      for name in ${inventoryShell}; do
        checkElf "$name" "$out/bin/$name"
      done

      runHook postInstallCheck
    '';
  };
in
{
  inherit package;
  d2bHostToolOverrides = lib.genAttrs overrideKeys (_: package);
}
