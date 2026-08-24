{
  description = "Opinionated NixOS desktop microVM workspaces";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # Package-only Rust build helper for shared host-tool dependency
    # artifacts. It is deliberately not used as an overlay.
    crane = {
      url = "github:ipetkov/crane";
    };

    # The contributor environment intentionally keeps its executable inputs
    # separate from the d2b substrate.  These are source-only inputs where
    # the package expression is the public surface; they must not become
    # overlays or alter the default module.
    gascity = {
      url = "github:gastownhall/gascity/6e0399fb970190a35c3e3d5d272a02becec55ffe";
      flake = false;
    };
    gascity-packs = {
      url = "github:gastownhall/gascity-packs/f3826035bb7de7c34621c2fdcd8620ab5a18bb08";
      flake = false;
    };
    llm-agents = {
      url = "github:numtide/llm-agents.nix/387989ee56d550d86d46d9458ad68a55b9e0ca3b";
    };
    # This input is deliberately package-only: the repository's main
    # nixpkgs input remains the source of all existing d2b outputs.
    nixpkgs-gas-city = {
      url = "github:NixOS/nixpkgs/f13ff45afd1bb73e640eaa08a7066dbed07e3238";
    };

    # `microvm` flake input DROPPED per ADR 0018.
    # The d2b NixOS substrate owns its per-VM evaluator via
    # `nixos-modules/vm-evaluator.nix` + `nixos-modules/vm-options.nix`.
    # Runner argv planning lives in the owning Provider crates; the broker
    # consumes the trusted bundle's prebuilt argv.

    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    home-manager,
    gascity,
    gascity-packs,
    llm-agents,
    nixpkgs-gas-city,
    ...
  }@inputs:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      nixpkgsFor = forAllSystems (system: import nixpkgs { inherit system; });
      gasCityNixpkgsFor =
        forAllSystems (system: import nixpkgs-gas-city { inherit system; });
      bazel920For = system:
        import ./pkgs/bazel-9.2.0 {
          pkgs = nixpkgsFor.${system};
        };
      bazelWorkerImageFor = system:
        import ./nix/bazel-worker-image.nix {
          pkgs = nixpkgsFor.${system};
          bazel = bazel920For system;
          inherit system;
        };

      # The current Gas City source and the package-only nixpkgs input both
      # require Go 1.26.5. Keep the package set explicit so a future update
      # of the d2b substrate cannot silently change the contributor binary.
      gascityFor = system:
        import ./pkgs/gascity {
          pkgs = gasCityNixpkgsFor.${system};
          source = gascity;
        };
      doltFor = system:
        import ./pkgs/dolt {
          pkgs = gasCityNixpkgsFor.${system};
        };
      beadsFor = system:
        import ./pkgs/beads {
          pkgs = gasCityNixpkgsFor.${system};
        };
      copilotFor = system: llm-agents.packages.${system}.copilot-cli;

      gasCityContributorFor = system:
        import ./nix/gas-city-contributor {
          pkgs = nixpkgsFor.${system};
          gascityPacksSrc = gascity-packs;
          gascity = gascityFor system;
          dolt = doltFor system;
          beads = beadsFor system;
          copilot = copilotFor system;
          go = (gasCityNixpkgsFor.${system}).go_1_26;
          bazel = (gasCityNixpkgsFor.${system}).bazel_9;
          gascityRevision =
            "6e0399fb970190a35c3e3d5d272a02becec55ffe";
          gascityPacksRevision =
            "f3826035bb7de7c34621c2fdcd8620ab5a18bb08";
          beadsRevision = "bf97b73749ac3ef2fca2365b54537ac041ad4293";
          llmAgentsRevision =
            "387989ee56d550d86d46d9458ad68a55b9e0ca3b";
          packageNixpkgsRevision =
            "f13ff45afd1bb73e640eaa08a7066dbed07e3238";
        };

      gasCityPackageSmokeFor = system:
        let
          gascity = gascityFor system;
          dolt = doltFor system;
          beads = beadsFor system;
          copilot = copilotFor system;
          gasCityContributor = gasCityContributorFor system;
          go = gasCityNixpkgsFor.${system}.go_1_26;
          bazel = gasCityNixpkgsFor.${system}.bazel_9;
        in
        import ./tests/unit/smoke/gas-city-package-smoke.nix {
          pkgs = nixpkgsFor.${system};
          inherit gasCityContributor;
          gascityRevision =
            "6e0399fb970190a35c3e3d5d272a02becec55ffe";
          gascityPacksRevision =
            "f3826035bb7de7c34621c2fdcd8620ab5a18bb08";
          beadsRevision = "bf97b73749ac3ef2fca2365b54537ac041ad4293";
          llmAgentsRevision =
            "387989ee56d550d86d46d9458ad68a55b9e0ca3b";
          packageNixpkgsRevision =
            "f13ff45afd1bb73e640eaa08a7066dbed07e3238";
          copilotVersion = copilot.version;
          gascityVersion = gascity.version;
          goVersion = go.version;
          bazelVersion = bazel.version;
          doltVersion = dolt.version;
          beadsVersion = beads.version;
        };

      providerElfShim = import ./nix/provider-elf-shim.nix;
      mkGuestRustPackagesSrc = pkgs:
        pkgs.runCommand "d2b-guest-rust-src" { } ''
          mkdir -p $out/packages
          cp -r ${./packages/d2b-realm-core} $out/packages/d2b-realm-core
          cp -r ${./packages/d2b-core} $out/packages/d2b-core
          cp -r ${./packages/d2b-contracts} $out/packages/d2b-contracts
          cp -r ${./packages/d2b-contracts-control} $out/packages/d2b-contracts-control
          cp -r ${./packages/d2b-contracts-resource} $out/packages/d2b-contracts-resource
          cp -r ${./packages/d2b-guestd} $out/packages/d2b-guestd
          cp -r ${./packages/d2b-exec-runner} $out/packages/d2b-exec-runner
          cp -r ${./packages/d2b-sk-frontend} $out/packages/d2b-sk-frontend
          cp ${./packages/Cargo.guest.lock} $out/packages/Cargo.lock
          chmod -R u+w $out/packages/d2b-core
          chmod -R u+w $out/packages/d2b-contracts
          chmod -R u+w $out/packages/d2b-contracts-control
          chmod -R u+w $out/packages/d2b-contracts-resource
          chmod -R u+w $out/packages/d2b-realm-core
          cp ${./tests/fixtures/guest-rust-workspace/d2b-contracts.Cargo.toml} \
            $out/packages/d2b-contracts/Cargo.toml
          cp ${./tests/fixtures/guest-rust-workspace/d2b-realm-core.Cargo.toml} \
            $out/packages/d2b-realm-core/Cargo.toml
          cp ${./tests/fixtures/guest-rust-workspace/d2b-core.Cargo.toml} \
            $out/packages/d2b-core/Cargo.toml
          cp ${./tests/fixtures/guest-rust-workspace/Cargo.toml} \
            $out/packages/Cargo.toml
        '';
    in
    {
      # The public surface area - populated incrementally by the
      # refactor plan. This wires `nixosModules.default` for real
      # after refactoring `host.nix`'s `{ inputs, ... }:`
      # module-arg into a closure-passed partial application (see
      # `./nixos-modules/default.nix` for the wiring + rationale).
      #
      # Downstream consumers:
      #
      #   imports = [ inputs.d2b.nixosModules.default ];
      #
      # Future work will populate the remaining surface:
      #   packages.<sys>       - patched cloud-hypervisor, crosvm, etc.
      #   templates.default    - `nix flake init -t github:vicondoa/d2b`
      #   checks.<sys>         - flake-eval CI gates
      #   lib                  - re-exported helpers (subnetIp, mkMac, …)
      nixosModules.default = import ./nixos-modules { inherit inputs; };
      # U4's contributor environment is a separate consumer module.  The
      # generic framework module above remains unchanged.
      nixosModules.gasCityContributor =
        import ./nixos-modules/gas-city-contributor {
          packageFor = gasCityContributorFor;
        };

      # Developer shell: everything the Layer-1 gates need, in one place.
      #
      # Without this each focused gate would provision its own toolchain.
      # Enter this shell once so Bazel, Cargo, Nix, and the policy tools use
      # the pinned versions throughout the fixed graph.
      #
      # rustup rather than pkgs.rustc: rust-toolchain.toml pins a
      # version nixpkgs does not carry (the pin is 1.97.0; this nixpkgs has
      # 1.95.0), and rustup reads that file itself. Once the nixpkgs input
      # advances far enough to supply the pinned release, rustup can be dropped
      # for pkgs.rustc/pkgs.cargo and the pin will be served natively.
      devShells = forAllSystems (system: let
        pkgs = nixpkgsFor.${system};
        gasCityContributor = gasCityContributorFor system;
        bazel920 = bazel920For system;
        bazelActionShell = pkgs.buildFHSEnv {
          name = "d2b-bazel-action-shell";
          executableName = "bash";
          targetPkgs = fhsPkgs: with fhsPkgs; [
            bash
            coreutils
            gnugrep
          ];
          runScript = "${pkgs.bash}/bin/bash";
        };
        mkBazelShellHook = testPath: ''
          export D2B_PROJECT_SHELL=d2b
          export D2B_BAZEL_BIN="${bazel920}/bin/bazel"
          export BAZEL_SH="''${BAZEL_SH:-${bazelActionShell}/bin/bash}"
          export D2B_SHELLCHECK_BIN="${pkgs.shellcheck}/bin/shellcheck"
          export D2B_BAZEL_TEST_PATH="${testPath}"
        '';
      in {
        default = pkgs.mkShell {
          name = "d2b-dev";
          packages = with pkgs; [
            # Toolchain. rustup resolves rust-toolchain.toml.
            bazel920
            gnumake
            rustup
            stdenv.cc
            # Compiler cache. The cargo configs route rustc through
            # .cargo/rustc-wrapper.sh, which uses this when present and plain
            # rustc when absent, so the shell never has to clear RUSTC_WRAPPER.
            sccache
            # Test and audit tooling the gates otherwise fetch per invocation.
            cargo-nextest
            cargo-deny
            cargo-audit
            # Shell and data tooling used by the gate scripts themselves.
            shellcheck
            jq
            ripgrep
            acl
          ];
          shellHook = ''
            ${mkBazelShellHook (pkgs.lib.makeBinPath [
              bazel920
              pkgs.bash
              pkgs.coreutils
              pkgs.findutils
              pkgs.gnugrep
              pkgs.gnused
              pkgs.git
              pkgs.gnumake
              pkgs.jq
              pkgs.rustup
              pkgs.shellcheck
            ])}
            export SCCACHE_DIR="''${SCCACHE_DIR:-$HOME/.cache/d2b-sccache}"
            echo "d2b dev shell: rust $(sed -n 's/.*channel = "\(.*\)".*/\1/p' rust-toolchain.toml) via rustup, sccache at $SCCACHE_DIR"
          '';
        };
        # Focused shell for the evaluation-only Nix-unit runner. Keeping this
        # output separate lets the target acquire only its locked external
        # tools instead of entering the full Rust development shell.
        nix-unit = pkgs.mkShellNoCC {
          name = "d2b-nix-unit";
          packages = with pkgs; [
            jq
          ];
        };
        # Focused U1 shell: the compatibility proof must use the exact
        # official Bazel release rather than an ambient or Gas City Bazel.
        # Only Bazel shell actions enter the standard FHS action shell;
        # Bazel itself and local tests stay in the caller's environment.
        bazel = pkgs.mkShellNoCC {
          name = "d2b-bazel-compat";
          packages = with pkgs; [
            bazel920
            bash
            coreutils
            findutils
            gawk
            git
            gnumake
            gnugrep
            gnused
            jq
            rustup
            shellcheck
          ];
          shellHook = ''
            ${mkBazelShellHook (pkgs.lib.makeBinPath [
              bazel920
              pkgs.bash
              pkgs.coreutils
              pkgs.findutils
              pkgs.gawk
              pkgs.git
              pkgs.gnumake
              pkgs.gnugrep
              pkgs.gnused
              pkgs.jq
              pkgs.rustup
              pkgs.shellcheck
            ])}
            echo "d2b Bazel compatibility shell: $(${bazel920}/bin/bazel --version)"
          '';
        };
        # Contributor shell: the closure is the only source of executable
        # inputs, so entering this shell does not depend on the host PATH.
        gas-city = pkgs.mkShell {
          name = "d2b-gas-city";
          packages = [ gasCityContributor ];
          shellHook = ''
            export GC_CONTRIBUTOR_ROOT="${gasCityContributor}/share/gas-city-contributor"
            export PATH="${gasCityContributor}/bin"
            echo "Gas City contributor shell: $GC_CONTRIBUTOR_ROOT"
          '';
        };
      });

      packages = forAllSystems (system: let
        pkgs = nixpkgsFor.${system};
        bazel920 = bazel920For system;
        gascity = gascityFor system;
        dolt = doltFor system;
        beads = beadsFor system;
        copilot = copilotFor system;
        gasCityContributor = gasCityContributorFor system;
        rustPackagesSrc = pkgs.runCommand "d2b-rust-src" { } ''
          mkdir -p $out/packages
          cp ${./Cargo.toml} $out/Cargo.toml
          cp ${./Cargo.lock} $out/Cargo.lock
          cp ${./deny.toml} $out/deny.toml
          cp -r ${./packages}/. $out/packages/
          mkdir -p $out/docs/reference/schemas/v3/providers
          cp ${./docs/reference/schemas/v3/providers/transport-azure-relay.transport-settings.json} \
            $out/docs/reference/schemas/v3/providers/transport-azure-relay.transport-settings.json
          cp ${./docs/reference/schemas/v3/providers/transport-vsock.transport-binding.json} \
            $out/docs/reference/schemas/v3/providers/transport-vsock.transport-binding.json
        '';
        rustWorkspace = args: pkgs.rustPlatform.buildRustPackage ({
          pname = "d2b-rust-workspace";
          version = "0.0.0-bootstrap";
          src = rustPackagesSrc;
          sourceRoot = "d2b-rust-src";
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
          };
          RUSTC_WRAPPER = "";
          SCCACHE_DIR = "";
        } // args);
        guestRustPackagesSrc = mkGuestRustPackagesSrc pkgs;
        cargoLock = {
          lockFile = ./packages/Cargo.guest.lock;
        };
        guestStaticPackage = packageName: binName:
          pkgs.pkgsStatic.rustPlatform.buildRustPackage {
            pname = "${binName}-static";
            version = "0.0.0-bootstrap";
            src = guestRustPackagesSrc;
            sourceRoot = "d2b-guest-rust-src/packages";
            inherit cargoLock;
            cargoBuildFlags = [ "--package" packageName "--bin" binName ];
            doCheck = false;
            RUSTC_WRAPPER = "";
            SCCACHE_DIR = "";
            nativeBuildInputs = [ pkgs.pkgsStatic.binutils ];
            postInstall = ''
              readelf=${pkgs.pkgsStatic.binutils.bintools}/bin/readelf
              bin="$out/bin/${binName}"
              test -x "$bin"
              "$readelf" -h "$bin" >/dev/null
              "$readelf" -l "$bin" > "$TMPDIR/${binName}.program-headers"
              if grep -q 'Requesting program interpreter' "$TMPDIR/${binName}.program-headers"; then
                echo "${binName}: unexpected ELF interpreter" >&2
                cat "$TMPDIR/${binName}.program-headers" >&2
                exit 1
              fi
              if "$readelf" -d "$bin" > "$TMPDIR/${binName}.dynamic" 2> "$TMPDIR/${binName}.dynamic.err"; then
                if grep -q '(NEEDED)' "$TMPDIR/${binName}.dynamic"; then
                  echo "${binName}: unexpected dynamic dependency" >&2
                  cat "$TMPDIR/${binName}.dynamic" >&2
                  exit 1
                fi
              elif ! grep -qi 'no dynamic section' "$TMPDIR/${binName}.dynamic.err"; then
                echo "${binName}: readelf -d failed unexpectedly" >&2
                cat "$TMPDIR/${binName}.dynamic.err" >&2
                exit 1
              fi
            '';
          };
        guestShellRunnerStatic =
          pkgs.pkgsStatic.rustPlatform.buildRustPackage {
            pname = "d2b-guest-shell-runner-static";
            version = "0.0.0-bootstrap";
            src = rustPackagesSrc;
            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
            };
            sourceRoot = "d2b-rust-src";
            cargoBuildFlags = [
              "--package" "d2b-guest-shell-runner"
              "--features" "real-libshpool"
            ];
            doCheck = false;
            RUSTC_WRAPPER = "";
            SCCACHE_DIR = "";
            nativeBuildInputs = [
              pkgs.pkgsStatic.binutils
              pkgs.pkgsStatic.rustPlatform.bindgenHook
            ];
            postInstall = ''
              readelf=${pkgs.pkgsStatic.binutils.bintools}/bin/readelf
              bin="$out/bin/d2b-guest-shell-runner"
              test -x "$bin"
              "$readelf" -h "$bin" >/dev/null
              "$readelf" -l "$bin" > "$TMPDIR/d2b-guest-shell-runner.program-headers"
              if grep -q 'Requesting program interpreter' "$TMPDIR/d2b-guest-shell-runner.program-headers"; then
                echo "d2b-guest-shell-runner: unexpected ELF interpreter" >&2
                cat "$TMPDIR/d2b-guest-shell-runner.program-headers" >&2
                exit 1
              fi
              if "$readelf" -d "$bin" > "$TMPDIR/d2b-guest-shell-runner.dynamic" 2> "$TMPDIR/d2b-guest-shell-runner.dynamic.err"; then
                if grep -q '(NEEDED)' "$TMPDIR/d2b-guest-shell-runner.dynamic"; then
                  echo "d2b-guest-shell-runner: unexpected dynamic dependency" >&2
                  cat "$TMPDIR/d2b-guest-shell-runner.dynamic" >&2
                  exit 1
                fi
              elif ! grep -qi 'no dynamic section' "$TMPDIR/d2b-guest-shell-runner.dynamic.err"; then
                echo "d2b-guest-shell-runner: readelf -d failed unexpectedly" >&2
                cat "$TMPDIR/d2b-guest-shell-runner.dynamic.err" >&2
                exit 1
              fi
            '';
          };
        # The canonical Provider package surface is present before semantic
        # Provider artifacts are implemented. These outputs compile each
        # Provider crate and publish only a scaffold marker; owning Provider
        # units replace the marker with their signed artifact layout.
        providerScaffoldPackage = packageName:
          rustWorkspace {
            pname = packageName;
            cargoBuildFlags = [ "--package" packageName ];
            doCheck = false;
            installPhase = ''
              runHook preInstall
              mkdir -p "$out/share/d2b/provider"
              printf '%s\n' "${packageName}" > "$out/share/d2b/provider/scaffold"
              runHook postInstall
            '';
          };
      in {
        manpages = pkgs.runCommand "d2b-manpages" { } ''
          install -Dm644 ${./docs/manpages/d2b.1} "$out/share/man/man1/d2b.1"
          ${pkgs.gzip}/bin/gzip -n -c ${./docs/manpages/d2b.1} > "$out/share/man/man1/d2b.1.gz"
        '';

        completions = pkgs.runCommand "d2b-completions" { } ''
          install -Dm644 ${./completions/d2b.bash} "$out/share/bash-completion/completions/d2b"
          install -Dm644 ${./completions/d2b.zsh}  "$out/share/zsh/site-functions/_d2b"
          install -Dm644 ${./completions/d2b.fish} "$out/share/fish/vendor_completions.d/d2b.fish"
        '';
        d2b-guestd-static = guestStaticPackage "d2b-guestd" "d2b-guestd";
        d2b-exec-runner-static =
          guestStaticPackage "d2b-exec-runner" "d2b-exec-runner";
        d2b-sk-frontend-static =
          guestStaticPackage "d2b-sk-frontend" "d2b-sk-frontend";
        d2b-guest-shell-runner-static = guestShellRunnerStatic;
        d2b-clipd = rustWorkspace {
          pname = "d2b-provider-clipboard-wayland";
          cargoBuildFlags = [
            "--package"
            "d2b-provider-clipboard-wayland"
            "--bin"
            "d2b-clipd"
          ];
          doCheck = false;
        };
        d2b-wayland-proxy = rustWorkspace {
          pname = "d2b-provider-display-wayland";
          cargoBuildFlags = [
            "--package"
            "d2b-provider-display-wayland"
            "--bin"
            "d2b-wayland-proxy"
          ];
          doCheck = false;
          meta.mainProgram = "d2b-wayland-proxy";
        };
        d2b-sk-waybar-helper = rustWorkspace {
          pname = "d2b-provider-notification-desktop";
          cargoBuildFlags = [
            "--package"
            "d2b-provider-notification-desktop"
            "--bin"
            "d2b-sk-waybar-helper"
          ];
          doCheck = false;
          meta.mainProgram = "d2b-sk-waybar-helper";
        };
        d2b-unsafe-local-helper = rustWorkspace {
          pname = "d2b-unsafe-local-helper";
          cargoBuildFlags = [
            "--package"
            "d2b-unsafe-local-helper"
            "--bin"
            "d2b-unsafe-local-helper"
          ];
          doCheck = false;
          meta.mainProgram = "d2b-unsafe-local-helper";
        };
        d2b-resource-compiler = rustWorkspace {
          pname = "d2b-resource-compiler";
          cargoBuildFlags = [
            "--package"
            "d2b-resource-compiler"
            "--bin"
            "d2b-resource-compiler"
          ];
          doCheck = false;
          meta.mainProgram = "d2b-resource-compiler";
        };
        d2b-provider-activation-nixos =
          providerScaffoldPackage "d2b-provider-activation-nixos";
        d2b-provider-audio-pipewire =
          providerScaffoldPackage "d2b-provider-audio-pipewire";
        d2b-provider-clipboard-wayland =
          providerScaffoldPackage "d2b-provider-clipboard-wayland";
        d2b-provider-display-wayland =
          providerScaffoldPackage "d2b-provider-display-wayland";
        d2b-provider-notification-desktop =
          providerScaffoldPackage "d2b-provider-notification-desktop";
        d2b-provider-runtime-azure-container-apps =
          providerScaffoldPackage "d2b-provider-runtime-azure-container-apps";
        d2b-provider-runtime-azure-virtual-machine =
          providerScaffoldPackage "d2b-provider-runtime-azure-virtual-machine";
        d2b-provider-runtime-cloud-hypervisor =
          providerScaffoldPackage "d2b-provider-runtime-cloud-hypervisor";
        d2b-provider-runtime-qemu-media =
          providerScaffoldPackage "d2b-provider-runtime-qemu-media";
        d2b-provider-shell-terminal =
          providerScaffoldPackage "d2b-provider-shell-terminal";
        d2b-provider-transport-azure-relay =
          providerScaffoldPackage "d2b-provider-transport-azure-relay";
        d2b-provider-transport-unix =
          providerScaffoldPackage "d2b-provider-transport-unix";
        d2b-provider-transport-vsock =
          providerScaffoldPackage "d2b-provider-transport-vsock";

        signoz = import ./pkgs/signoz { inherit pkgs; };
        signozOtelCollector = import ./pkgs/signoz-otel-collector { inherit pkgs; };
        signozSchemaMigrator = import ./pkgs/signoz-schema-migrator { inherit pkgs; };
        bazel-9_2_0 = bazel920;
        bazel-worker-image = bazelWorkerImageFor system;
        inherit gascity dolt beads copilot gasCityContributor;
        gas-city-contributor = gasCityContributor;
      });

      # Container-based integration test images (the type-G layer), built by
      # Nix and run with podman, rootless. Exposed under `containerImages`,
      # NOT `checks`, so the Layer-1 `nix flake check --no-build --all-systems`
      # never builds an image. The `make test-integration` target
      # (tests/integration/containers/*.sh, driven via podman) builds + runs them; the same
      # target runs on a GitHub Actions ubuntu-latest job (podman is
      # preinstalled there) and locally.
      #
      # Scope: this layer is ONLY for things that need a foreign (non-Nix)
      # userland - e.g. proving a static d2b binary runs on stock Ubuntu.
      # It deliberately does NOT boot systemd for daemon/socket activation;
      # that is covered natively by
      # packages/d2b-priv-broker/tests/socket_activation.rs plus nix-unit.
      # See tests/integration/containers/README.md.
      #
      # Auto-discovered from tests/integration/containers/images/*.nix: each image module is
      # `{ pkgs, self, system }: <dockerTools-built OCI image>`, so adding a new
      # container test is one new image file + its tests/integration/containers/<name>.sh
      # runner - no edit here. x86_64-linux only (the project's CI runners +
      # this host are x86_64; aarch64 images need an aarch64 builder).
      containerImages = forAllSystems (system:
        if system == "x86_64-linux" then
          let
            pkgs = nixpkgsFor.${system};
            imageDir = ./tests/integration/containers/images;
            imageFiles = if builtins.pathExists imageDir
              then builtins.attrNames (nixpkgs.lib.filterAttrs
                (name: type: type == "regular" && nixpkgs.lib.hasSuffix ".nix" name)
                (builtins.readDir imageDir))
              else [ ];
            mkImage = file: {
              name = nixpkgs.lib.removeSuffix ".nix" file;
              value = import (imageDir + "/${file}") { inherit pkgs self system; };
            };
          in builtins.listToAttrs (map mkImage imageFiles)
        else { });

      # Type-G runNixOSTest integration tests (the additive real-kernel
      # coverage layer). Each test boots a real NixOS VM with the d2b
      # daemon surface and asserts live broker/daemon/host-posture behaviour
      # (socket activation, SO_PEERCRED, bridge isolation, state-dir ACLs,
      # broker privilege posture) that the fake-backed native Rust canaries and
      # pure-eval gates cannot exercise. This is the hermetic, non-destructive
      # successor to the `D2B_LIVE`-against-the-real-host bash scripts.
      #
      # Exposed under `vmChecks`, NOT `checks`, so the Layer-1 `nix flake check
      # --no-build --all-systems` never realizes a VM. Selected explicitly by
      # `make test-host-integration` (`nix build .#vmChecks.<system>.<name>`),
      # which needs KVM (a local NixOS host; TCG fallback otherwise).
      #
      # Auto-discovered from tests/host-integration/*.nix (excluding lib.nix): each test is
      # `{ pkgs, self }: pkgs.testers.runNixOSTest { ... }`, so adding a VM test
      # is one new file - no edit here. x86_64-linux only: a runNixOSTest VM is
      # built + booted for the builder's own system, and the hosted CI runners
      # are x86_64 - aarch64 VM coverage needs an aarch64 builder.
      vmChecks = forAllSystems (system:
        if system == "x86_64-linux" then
          let
            pkgs = nixpkgsFor.${system};
            hostToolBundleEnv = builtins.getEnv "D2B_HOST_TOOL_BUNDLE";
            bazelHostTools =
              if hostToolBundleEnv == "" then
                null
              else
                import ./nix/test-support/bazel-host-tools.nix {
                  inherit pkgs;
                  rawBundle = builtins.storePath hostToolBundleEnv;
                };
            hostToolOverrides =
              if bazelHostTools == null
              then null
              else bazelHostTools.d2bHostToolOverrides;
            testSelf =
              if bazelHostTools == null then
                self
              else
                self // {
                  nixosModules = self.nixosModules // {
                    default = {
                      imports = [ self.nixosModules.default ];
                      _module.args.d2bHostToolOverrides = hostToolOverrides;
                    };
                  };
                  packages = self.packages // {
                    ${system} = self.packages.${system} // {
                      d2b-wayland-proxy = bazelHostTools.package;
                    };
                  };
                };
            testDir = ./tests/host-integration;
            testFiles = if builtins.pathExists testDir
              then builtins.attrNames (nixpkgs.lib.filterAttrs
                (name: type:
                  type == "regular"
                  && nixpkgs.lib.hasSuffix ".nix" name
                  && name != "lib.nix")
                (builtins.readDir testDir))
              else [ ];
            mkTest = file: {
              name = nixpkgs.lib.removeSuffix ".nix" file;
              value = import (testDir + "/${file}") {
                inherit pkgs;
                self = testSelf;
              };
            };
          in builtins.listToAttrs (map mkTest testFiles)
        else { });

      templates.default = {
        path = ./templates/default;
        description = "Minimal d2b host scaffold - one env, one headless workload VM";
      };

      # Eval-only gates for the in-tree examples + template. The
      # `system.build.toplevel.drvPath` access is enough to force a
      # full module-system instantiation (option types, assertions,
      # CIDR validators, etc.) without actually realising the closure
      # - which is what we want from a `nix flake check` gate.
      #
      # `with-entra-id` is intentionally absent: it imports
      # `entrablau.nixosModules.default` from a separate sibling
      # flake, and the root flake doesn't (and shouldn't) pull that
      # in as an input. The example's own `flake.nix` still gates
      # eval via `nix flake check` in its own directory; the
      # The fixed flake evaluation lane exercises it.
      #
      # The template's `configuration.nix` carries sentinel
      # assertions that fail eval until the operator replaces
      # placeholder values (TODOs 2/3). To eval-check the template
      # without disturbing those assertions for real users, we layer
      # a third module on top that uses `lib.mkForce` to replace
      # just the sentinel-gated fields with valid stand-ins. Sentinel
      # detection logic stays in the template; the override is
      # local to this check.
      checks = forAllSystems (system: let
        pkgs = nixpkgsFor.${system};
        bazel920 = bazel920For system;
        d2bModule = import ./nixos-modules { inherit inputs; };
        mkEval = modules: nixpkgs.lib.nixosSystem {
          inherit system;
          modules = [
            d2bModule
            ({ lib, ... }: {
              # Cross-system eval cannot use x86-only release prebuilts.
              # Native x86 eval keeps the consumer default to avoid forcing
              # source host-tool derivations through every lightweight check.
              d2b.site.usePrebuiltHostTools = lib.mkDefault (system == "x86_64-linux");
            })
          ] ++ modules;
        };
        mkCheck = name: cfg: pkgs.runCommand "d2b-check-${name}" { } ''
          echo ${builtins.unsafeDiscardStringContext cfg.config.system.build.toplevel.drvPath} > $out
        '';
        mkEvalOnlyCheck = name: value: pkgs.runCommand "d2b-check-${name}" { } ''
          echo ${builtins.unsafeDiscardStringContext (builtins.toJSON value)} > $out
        '';
        smokeConfigModule = { lib, ... }: {
          boot.loader.grub.enable = false;
          boot.loader.systemd-boot.enable = false;
          boot.initrd.includeDefaultModules = false;
          fileSystems."/" = {
            device = "tmpfs";
            fsType = "tmpfs";
          };
          environment.etc."machine-id".text =
            "00000000000000000000000000000000";
          system.stateVersion = "25.11";

          users.users.alice = {
            isNormalUser = true;
            uid = 1000;
          };

          d2b.site = {
            waylandUser = "alice";
            launcherUsers = [ "alice" ];
            yubikey.enable = false;
          };

          d2b.envs.work = {
            lanSubnet = "10.20.0.0/24";
            uplinkSubnet = "192.0.2.0/30";
          };

          d2b.vms.corp-vm = {
            enable = true;
            env = "work";
            index = 10;
            ssh.user = "alice";
            config = {
              networking.hostName = lib.mkDefault "corp-vm";
              users.users.alice = {
                isNormalUser = true;
                uid = 1000;
              };
            };
          };

          d2b.realms.host = {
            allowedUsers = [ "alice" ];
            policy.allowUnsafeLocal = true;
            network.ui.accentColor = "#cc3344";
            workloads.tools = {
              kind = "unsafe-local";
              shell = {
                enable = true;
                defaultName = "host";
                maxSessions = 8;
              };
              launcher = {
                enable = true;
                label = "Local tools";
                defaultItem = "browser";
                items = {
                  browser = {
                    type = "exec";
                    name = "Browser";
                    icon.name = "firefox";
                    argv = [ "firefox" "rendered-private-argv-canary" ];
                    graphical = true;
                  };
                  terminal = {
                    type = "shell";
                    name = "Terminal";
                    icon.name = "terminal";
                  };
                };
              };
            };
          };
        };
        # The eval-only fixtures contain no authored v3 artifacts. Keep their
        # catalog projection deterministic instead of forcing the production
        # artifact-catalog IFD (`runCommand` + `builtins.readFile`) while
        # rendering an otherwise unrelated VM fixture. The production module
        # remains the authority for real configurations with authored
        # artifacts; this is only the fixture boundary for the empty case.
        fixtureArtifactCatalogData = {
          schemaVersion = 3;
          entries = [ ];
        };
        fixtureArtifactCatalogPreimageJson =
          builtins.toJSON fixtureArtifactCatalogData;
        fixtureArtifactCatalogDigest = "sha256:${builtins.hashString
          "sha256"
          (builtins.toJSON {
            domain = "d2b:v3:artifact-catalog";
            framing = "d2b-digest/v1";
            payload = fixtureArtifactCatalogPreimageJson;
          })}";
        fixtureArtifactCatalogDocument = fixtureArtifactCatalogData // {
          catalogDigest = fixtureArtifactCatalogDigest;
        };
        fixtureArtifactCatalogJson =
          builtins.toJSON fixtureArtifactCatalogDocument;
        fixtureArtifactCatalogPath = pkgs.writeText
          "d2b-artifact-catalog-eval-fixture.json"
          "${fixtureArtifactCatalogJson}\n";
        fixtureArtifactCatalogProjection = {
          ids = [ ];
          artifactRows = [ ];
          preimage = fixtureArtifactCatalogData;
          preimageJson = fixtureArtifactCatalogPreimageJson;
          catalogDigest = fixtureArtifactCatalogDigest;
          catalogData = fixtureArtifactCatalogDocument;
          catalogJson = fixtureArtifactCatalogJson;
          path = fixtureArtifactCatalogPath;
          publicEntries = [ ];
        };
        fixtureArtifactCatalogArtifact = {
          data = fixtureArtifactCatalogData;
          jsonText = fixtureArtifactCatalogJson;
          path = fixtureArtifactCatalogPath;
          installFileName = "artifact-catalog.json";
          classification = "contractPrivateNonSecret";
          sensitivity = "nonSecret";
        };
        fixtureArtifactCatalogOverride = { lib, ... }: {
          d2b._artifactCatalogV3 = lib.mkForce
            fixtureArtifactCatalogProjection;
          d2b._bundle.extraArtifacts.artifactCatalog =
            lib.mkOverride 0 fixtureArtifactCatalogArtifact;
        };
        smokeEval = mkEval [
          smokeConfigModule
          ({ lib, ... }: {
            # Contract fixtures must render the just-built workspace tools.
            # Release prebuilts may not exist for unreleased development
            # versions, and using prebuilts would hide changes to runner argv
            # and helper paths from the rendered artifact tests.
            d2b.site.usePrebuiltHostTools = lib.mkForce false;
          })
          fixtureArtifactCatalogOverride
        ];
        renderEvalFixture = {
          evaluated
        , includeClosures ? true
        , processData ? null
        }: let
          bundle = evaluated.config.d2b._bundle;
          top = name: bundle.${name}.fixtureData;
        in {
          files = {
            "privileges.json" = top "privilegesJson";
            "host.json" = top "hostJson";
            "processes.json" =
              if processData == null then top "processesJson" else processData;
            "storage.json" = top "storageJson";
            "sync.json" = top "syncJson";
            "allocator.json" = top "allocatorJson";
            "realm-controllers.json" = top "realmControllersJson";
            "realm-identity.json" = top "realmIdentityJson";
            "realm-workloads-launcher.json" = top "realmWorkloadsLauncherJson";
            "realm-workloads-launcher-v2.json" = top "realmWorkloadsLauncherV2Json";
            "unsafe-local-workloads.json" = top "unsafeLocalWorkloadsJson";
            "bundle.json" = top "bundle";
            "manifest.json" = evaluated.config.d2b._manifestData;
          };
          closures = if includeClosures
            then pkgs.lib.mapAttrs (_: closure: closure.data) bundle.closures
            else { };
        };
        smokeFixture = let
          bundle = smokeEval.config.d2b._bundle;
          manifestPkg = smokeEval.config.d2b._manifestPkg;
        in pkgs.runCommand "d2b-fixture-smoke" { } ''
          mkdir -p $out $out/closures
          cp ${bundle.privilegesJson.path} $out/privileges.json
          cp ${bundle.hostJson.path} $out/host.json
          cp ${bundle.processesJson.path} $out/processes.json
          cp ${bundle.storageJson.path} $out/storage.json
          cp ${bundle.syncJson.path} $out/sync.json
          cp ${bundle.allocatorJson.path} $out/allocator.json
          cp ${bundle.realmControllersJson.path} $out/realm-controllers.json
          cp ${bundle.realmIdentityJson.path} $out/realm-identity.json
          cp ${bundle.realmWorkloadsLauncherJson.path} $out/realm-workloads-launcher.json
          cp ${bundle.realmWorkloadsLauncherV2Json.path} $out/realm-workloads-launcher-v2.json
          cp ${bundle.unsafeLocalWorkloadsJson.path} $out/unsafe-local-workloads.json
          cp ${bundle.bundle.path} $out/bundle.json
          cp ${manifestPkg}/share/d2b/vms.json $out/manifest.json
          ${nixpkgs.lib.concatStringsSep "\n" (nixpkgs.lib.mapAttrsToList
            (vm: c: "cp ${c.path} $out/closures/${vm}.json")
            bundle.closures)}
        '';
        # Feature-RICH fixture: a single workload VM with graphics + video +
        # audio + tpm + usbip + observability enabled, so every per-role
        # minijail profile (gpu, wayland-proxy, video, audio, swtpm, usbip,
        # vsock-relay, otel-host-bridge) renders into the bundle. Consumed by
        # the per-role minijail-validator contract tests. x86_64-linux only:
        # the framework's checkVmPlatform gate throws on graphics for aarch64,
        # so this is referenced only under that guard below (lazily - never
        # forced on aarch64).
        fullConfigModule = { lib, ... }: {
          boot.loader.grub.enable = false;
          boot.loader.systemd-boot.enable = false;
          boot.initrd.includeDefaultModules = false;
          fileSystems."/" = {
            device = "tmpfs";
            fsType = "tmpfs";
          };
          environment.etc."machine-id".text =
            "00000000000000000000000000000000";
          system.stateVersion = "25.11";

          users.users.alice = {
            isNormalUser = true;
            uid = 1000;
          };

          d2b.site = {
            waylandUser = "alice";
            launcherUsers = [ "alice" ];
            yubikey.enable = true;
          };

          d2b.observability.enable = true;

          d2b.envs.work = {
            lanSubnet = "10.20.0.0/24";
            uplinkSubnet = "192.0.2.0/30";
          };

          d2b.vms.corp-full = {
            enable = true;
            env = "work";
            index = 10;
            ssh.user = "alice";
            graphics.enable = true;
            graphics.crossDomainTrusted = true;
            graphics.videoSidecar = true;
            audio.enable = true;
            usbip.yubikey = true;
            guest.control.enable = true;
            tpm.enable = true;
            observability.enable = true;
            config = {
              networking.hostName = lib.mkDefault "corp-full";
              users.users.alice = {
                isNormalUser = true;
                uid = 1000;
              };
            };
          };
        };
        fullEval = mkEval [
          fullConfigModule
          ({ lib, ... }: {
            # See smokeEval above: the feature-rich fixture is a rendered
            # contract oracle, so it must consume source-built host tools.
            d2b.site.usePrebuiltHostTools = lib.mkForce false;
          })
          fixtureArtifactCatalogOverride
        ];
        # The eval-rendered full fixture validates the serialized runner and
        # minijail contracts, not the guest kernel or hypervisor binaries.
        # Keep those package edges deterministic and narrow in this fixture
        # only. The real `fullEval` remains the source for the realized video
        # command-surface check and the explicit full fixture derivation.
        fixtureKernel = {
          dev = pkgs.runCommand "linux-6.18.33-dev" { } ''
            mkdir -p "$out"
            touch "$out/vmlinux"
          '';
          out = pkgs.runCommand "linux-6.18.33" { } ''
            mkdir -p "$out"
          '';
        };
        fixtureInitrd = pkgs.runCommand "initrd-linux-6.18.33" { } ''
          mkdir -p "$out"
          touch "$out/initrd"
        '';
        fixtureVmPackage = name:
          pkgs.writeShellScriptBin name "exit 0";
        fullFixtureVmTools = { lib, ... }: {
          d2b.vms.corp-full.config.microvm = {
            kernel = lib.mkForce fixtureKernel;
            initrdPath = lib.mkForce "${fixtureInitrd}/initrd";
            cloud-hypervisor.package = lib.mkForce
              (fixtureVmPackage "cloud-hypervisor");
            virtiofsd.package = lib.mkForce
              (fixtureVmPackage "virtiofsd");
            graphics.crosvmPackage = lib.mkForce
              (fixtureVmPackage "crosvm");
          };
        };
        fullEvalFixture = mkEval [
          fullConfigModule
          fullFixtureVmTools
          ({ lib, ... }: {
            d2b.site.usePrebuiltHostTools = lib.mkForce false;
          })
          fixtureArtifactCatalogOverride
        ];
        fullProcessFixtureData =
          let
            data = fullEvalFixture.config.d2b._bundle.processesJson.fixtureData;
          in
          data // {
            # Full contract consumers need the feature VM, the env's usbipd
            # backend/proxy, and the observability host bridge. They do not
            # consume the auto-declared net DAG, so do not force that
            # unrelated runner subgraph through the eval-only projection.
            vms = pkgs.lib.map
              (dag:
                if dag.vm == "sys-obs" then
                  dag // {
                    nodes = pkgs.lib.filter
                      (node: node.id == "otel-host-bridge")
                      dag.nodes;
                  }
                else
                  dag)
              (pkgs.lib.filter
                (dag:
                  builtins.elem dag.vm
                    [ "corp-full" "sys-work-usbipd" "sys-obs" ])
                data.vms);
          };
        fullFixture = let
          bundle = fullEval.config.d2b._bundle;
          manifestPkg = fullEval.config.d2b._manifestPkg;
        in pkgs.runCommand "d2b-fixture-smoke-full" { } ''
          mkdir -p $out
          cp ${bundle.privilegesJson.path} $out/privileges.json
          cp ${bundle.hostJson.path} $out/host.json
          cp ${bundle.processesJson.path} $out/processes.json
          cp ${bundle.storageJson.path} $out/storage.json
          cp ${bundle.syncJson.path} $out/sync.json
          cp ${bundle.allocatorJson.path} $out/allocator.json
          cp ${bundle.realmControllersJson.path} $out/realm-controllers.json
          cp ${bundle.realmIdentityJson.path} $out/realm-identity.json
          cp ${bundle.bundle.path} $out/bundle.json
          cp ${manifestPkg}/share/d2b/vms.json $out/manifest.json
        '';
        evalFixtureData = {
          minimal = renderEvalFixture {
            evaluated = smokeEval;
          };
          # Full fixture consumers validate feature-specific bundle/process
          # contracts only. They do not consume closure JSON, so do not force
          # the VM closure graph back through this eval-only surface.
          full = renderEvalFixture {
            evaluated = fullEvalFixture;
            includeClosures = false;
            processData = fullProcessFixtureData;
          };
        };
        fullProcessDags = fullEval.config.d2b._bundle.processesJson.data.vms;
        fullCorpDag = pkgs.lib.findFirst (dag: dag.vm == "corp-full")
          (throw "video binary contract: corp-full DAG missing") fullProcessDags;
        fullVideoNode = pkgs.lib.findFirst (node: node.id == "video")
          (throw "video binary contract: video node missing") fullCorpDag.nodes;
        fullCloudHypervisorNode = pkgs.lib.findFirst (node: node.id == "cloud-hypervisor")
          (throw "video binary contract: cloud-hypervisor node missing") fullCorpDag.nodes;
        videoBinaryContract = pkgs.runCommand "d2b-video-binary-command-surface"
          { nativeBuildInputs = [ pkgs.gnugrep ]; } ''
          set -euo pipefail
          test -x ${fullVideoNode.binaryPath}
          test -x ${fullCloudHypervisorNode.binaryPath}
          video_help=$(${fullVideoNode.binaryPath} device video-decoder --help 2>&1)
          printf '%s\n' "$video_help" | grep -F -- --backend
          vmm_help=$(${fullCloudHypervisorNode.binaryPath} --help 2>&1)
          printf '%s\n' "$vmm_help" | grep -F -- --vhost-user-media
          touch "$out"
        '';
        # Rust tests reach repo-level fixtures under tests/golden/
        # (compile-time
        # include_str! goldens) and tests/fixtures/ (compile-time +
        # runtime fixture-path reads from unit/integration tests).
        # Compose a sandbox src that holds packages/, the runtime schemas
        # embedded by provider crates, plus those fixture
        # trees so the cargo workspace never reads outside its packaged
        # source in the Nix sandbox. Operators running cargo OUTSIDE
        # the sandbox use the raw ./packages tree and the same relative
        # paths still resolve against the checkout.
        rustPackagesSrc = pkgs.runCommand "d2b-rust-src" { } ''
          mkdir -p $out/packages
          cp ${./Cargo.toml} $out/Cargo.toml
          cp ${./Cargo.lock} $out/Cargo.lock
          cp ${./deny.toml} $out/deny.toml
          cp -r ${./packages}/. $out/packages/
          mkdir -p $out/docs/reference/schemas/v3/providers
          cp ${./docs/reference/schemas/v3/providers/transport-azure-relay.transport-settings.json} \
            $out/docs/reference/schemas/v3/providers/transport-azure-relay.transport-settings.json
          cp ${./docs/reference/schemas/v3/providers/transport-vsock.transport-binding.json} \
            $out/docs/reference/schemas/v3/providers/transport-vsock.transport-binding.json
          mkdir -p $out/tests
          cp -r ${./tests/golden} $out/tests/golden
          cp -r ${./tests/fixtures} $out/tests/fixtures
        '';
        guestRustPackagesSrc = mkGuestRustPackagesSrc pkgs;
        rustWorkspace = args: pkgs.rustPlatform.buildRustPackage ({
          pname = "d2b-rust-workspace";
          version = "0.0.0-bootstrap";
          src = rustPackagesSrc;
          sourceRoot = "d2b-rust-src";
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
          };
          # Repo-local .cargo/config.toml files set
          # `rustc-wrapper = "sccache"`, but the Nix sandbox doesn't
          # have sccache on PATH (and even if it did, sccache wants
          # a writable cache dir + network for distributed builds).
          # Disable the wrapper for sandbox builds. Operators running
          # cargo OUTSIDE the sandbox (worktrees, dev shells) still
          # get the sccache speedup from the config files.
          RUSTC_WRAPPER = "";
          SCCACHE_DIR = "";
        } // args);
        rustToolchainChannel =
          (builtins.fromTOML (builtins.readFile ./rust-toolchain.toml)).toolchain.channel;
        brokerManifestToml = builtins.fromTOML (builtins.readFile ./packages/d2b-priv-broker/Cargo.toml);
        mainManifestToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
        assertRustToolchain = ''
          rustc --version | grep -F "${rustToolchainChannel}"
        '';
        assertRustSupplyChainInputs = ''
          test -f ${rustPackagesSrc}/Cargo.lock
          test -f ${rustPackagesSrc}/packages/Cargo.guest.lock
          test -f ${rustPackagesSrc}/deny.toml
          printf '%s\n' '${builtins.toJSON mainManifestToml.workspace.members}' >/dev/null
          printf '%s\n' '${brokerManifestToml.package.name}' >/dev/null
        '';

        # Pinned RustSec advisory DB snapshot for offline cargo-deny /
        # cargo-audit checks in the Nix sandbox.  Update the rev + hash
        # periodically to pick up new advisories.
        advisoryDbSrc = pkgs.fetchFromGitHub {
          owner = "rustsec";
          repo = "advisory-db";
          rev = "831c50f4a4304068f125e603add6a8839f08b3eb";
          hash = "sha256-wXKYURZz76ZC5lbuDA1oVQA/MxSB3pSJ1raF1HG0oIc=";
        };

        # cargo-deny and cargo-audit (via the rustsec crate) require the
        # advisory DB to be a git repository.  Wrap the fetchFromGitHub
        # source tree in a minimal git repo so gix::open succeeds.
        advisoryDbGit = pkgs.runCommand "rustsec-advisory-db-git" {
          nativeBuildInputs = [ pkgs.git ];
        } ''
          cp -r ${advisoryDbSrc} $out
          chmod -R u+w $out
          cd $out
          git init -q
          git add .
          git -c user.email=nixbld@localhost -c user.name=nixbld \
            commit -q -m 'advisory-db snapshot'
        '';

      in {
        eval-fixture-contracts =
          if system == "x86_64-linux" then
            (mkEvalOnlyCheck "eval-fixture-contracts" evalFixtureData) // {
              fixtureData = evalFixtureData;
            }
          else
            (pkgs.runCommand "d2b-eval-fixture-contracts-unsupported" { } ''
              echo "eval-fixture-contracts is x86_64-linux only (graphics gate)" > $out
            '') // {
              fixtureData = { };
            };
        video-binary-contract =
          if system == "x86_64-linux" then
            videoBinaryContract
          else
            pkgs.runCommand "d2b-video-binary-contract-unsupported" { } ''
              echo "video-binary-contract is x86_64-linux only (graphics gate)" > $out
            '';
        fixture-smoke = smokeFixture;
        # Unlike the existing eval-only fixture checks, this one deliberately
        # realizes every pinned executable and the immutable pack closure.
        gas-city-package-smoke = gasCityPackageSmokeFor system;
        bazel-9_2_0-provider-smoke =
          import ./tests/unit/smoke/bazel-provider.nix {
            inherit pkgs bazel920 system;
          };

        # Feature-rich fixture for the per-role minijail-validator contract
        # tests. x86_64-linux only (graphics platform gate); on other systems
        # the key resolves to a trivial derivation so `nix flake check
        # --all-systems` never forces the graphics eval.
        fixture-smoke-full =
          if system == "x86_64-linux" then
            fullFixture
          else
            pkgs.runCommand "d2b-fixture-smoke-full-unsupported" { } ''
              echo "fixture-smoke-full is x86_64-linux only (graphics gate)" > $out
            '';

        eval-minimal = mkCheck "eval-minimal"
          (mkEval [ (import ./examples/minimal/configuration.nix) ]);

        eval-multi-env = mkCheck "eval-multi-env"
          (mkEval [ (import ./examples/multi-env/configuration.nix) ]);

        eval-multi-env-daemon = mkCheck "eval-multi-env-daemon"
          (mkEval [
            (import ./examples/multi-env/configuration.nix)
            ({ lib, ... }: {
              d2b.site.allowUnsafeEastWest = true;
              d2b.daemonExperimental.enable = true;
              d2b.envs.work.mtu = lib.mkForce 1400;
              d2b.envs.work.mssClamp = lib.mkForce true;
              d2b.envs.work.lan.allowEastWest = lib.mkForce true;
            })
          ]);

        eval-with-observability =
          let
            cfg = mkEval [ (import ./examples/with-observability/configuration.nix) ];
            observed = {
              assertionsGreen = pkgs.lib.all (a: a.assertion) cfg.config.assertions;
              observabilityEnabled =
                (builtins.fromJSON cfg.config.d2b._manifestPkg.text)._observability.enabled;
              stackVmDeclared = builtins.hasAttr "sys-obs" cfg.config.d2b.vms;
              workloadAgentDeclared =
                cfg.config.d2b.vms.work-app.observability.enable;
            };
          in
          mkEvalOnlyCheck "eval-with-observability" (
            if observed.assertionsGreen
              && observed.observabilityEnabled
              && observed.stackVmDeclared
              && observed.workloadAgentDeclared
            then observed
            else throw "eval-with-observability failed: ${builtins.toJSON observed}"
          );

        rust-build = rustWorkspace {
          pname = "d2b-rust-build";
          preBuild = assertRustToolchain;
          cargoBuildFlags = [ "--workspace" ];
          doCheck = false;
        };

        rust-tests = rustWorkspace {
          pname = "d2b-rust-tests";
          preBuild = assertRustToolchain;
          cargoBuildFlags = [ "--workspace" ];
          cargoTestFlags = [ "--workspace" ];
          installPhase = ''
            runHook preInstall
            mkdir -p $out
            echo ok > $out/rust-tests
            runHook postInstall
          '';
        };

        rust-clippy = rustWorkspace {
          pname = "d2b-rust-clippy";
          nativeBuildInputs = [ pkgs.clippy ];
          cargoBuildFlags = [ "--workspace" ];
          doCheck = false;
          buildPhase = ''
            runHook preBuild
            ${assertRustToolchain}
            cargo clippy --workspace --all-targets -- -D warnings
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            mkdir -p $out
            echo ok > $out/rust-clippy
            runHook postInstall
          '';
        };

        guest-static-elf = import ./tests/unit/smoke/guest-static-elf.nix {
          inherit system pkgs;
          flake = self;
        };

        # Build-level determinism proof for the Provider package catalog
        # emitter. The drift gate proves the generator's output matches what is
        # committed; only this proves it emits the same bytes across two
        # independent evaluations of the same input. The eval file throws on a
        # mismatch, so `nix flake check --no-build` fails at evaluation rather
        # than producing an unbuilt derivation.
        provider-catalog-determinism = let
          evidence = import ./tests/unit/smoke/provider-catalog-determinism-eval.nix {
            inherit system pkgs;
            flake = self;
          };
        in pkgs.runCommand "d2b-provider-catalog-determinism" { } ''
          mkdir -p "$out"
          printf '%s\n' '${evidence}' > "$out/provider-catalog-determinism.json"
        '';

        guest-static-consumption = let
          evidence = import ./tests/unit/smoke/guest-static-consumption-eval.nix {
            inherit system pkgs;
            flake = self;
          };
        in pkgs.runCommand "d2b-guest-static-consumption" { } ''
          mkdir -p "$out"
          printf '%s\n' '${evidence}' > "$out/guest-static-consumption.json"
        '';

        # Real cargo-deny gate: bans, licenses, and sources for the
        # repository-root product workspace. Advisory checks are handled by
        # rust-audit below (cargo-deny requires
        # a fetchable URL for the advisory DB which is incompatible
        # with the Nix sandbox's no-network constraint).
        #
        # cargo-deny shells out to `cargo metadata`, so we vendor
        # the crate registry and override the sccache wrapper that
        # the repo-local .cargo/config.toml enables.
        rust-deny = let
          mainVendor = pkgs.rustPlatform.importCargoLock {
            lockFile = ./Cargo.lock;
            outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
          };
          cargoConfig = vendorDir: ''
            [source.crates-io]
            replace-with = "vendored-sources"
            [source."git+https://github.com/vicondoa/wl-proxy.git?rev=072945b59fef21a2a8166460454280d543f48772#072945b59fef21a2a8166460454280d543f48772"]
            git = "https://github.com/vicondoa/wl-proxy.git"
            rev = "072945b59fef21a2a8166460454280d543f48772"
            replace-with = "vendored-sources"
            [source.vendored-sources]
            directory = "${vendorDir}"
          '';
        in pkgs.runCommand "d2b-rust-deny" {
          nativeBuildInputs = [ pkgs.cargo-deny pkgs.cargo pkgs.rustc ];
        } ''
          export HOME="$TMPDIR"

          run_deny() {
            local label=$1 src=$2 manifest=$3 vendor_cfg=$4 deny_cfg=$5
            local ws="$TMPDIR/$label"
            cp -r "$src/." "$ws"
            chmod -R u+w "$ws"
            # Override all .cargo/config.toml files to disable sccache
            # and enable vendored dependencies.
            find "$ws" -path '*/.cargo/config.toml' -exec sh -c \
              'printf "%s\n" "$1" > "$0"' {} "$vendor_cfg" \;
            mkdir -p "$ws/.cargo"
            printf '%s\n' "$vendor_cfg" > "$ws/.cargo/config.toml"
            echo "==> cargo deny check ($label)"
            cargo-deny --manifest-path "$ws/$manifest" \
              check --config "$deny_cfg" bans licenses sources
            rm -rf "$ws"
          }

          run_deny "main" \
            "${rustPackagesSrc}" \
            "Cargo.toml" \
            '${cargoConfig mainVendor}' \
            "${rustPackagesSrc}/deny.toml"

          echo ok > $out
        '';

        guest-rust-deny = let
          guestVendor = pkgs.rustPlatform.importCargoLock {
            lockFile = ./packages/Cargo.guest.lock;
          };
          cargoConfig = ''
            [source.crates-io]
            replace-with = "vendored-sources"
            [source.vendored-sources]
            directory = "${guestVendor}"
          '';
        in pkgs.runCommand "d2b-guest-rust-deny" {
          nativeBuildInputs = [ pkgs.cargo-deny pkgs.cargo pkgs.rustc ];
        } ''
          export HOME="$TMPDIR"
          ws="$TMPDIR/guest"
          cp -r "${guestRustPackagesSrc}/packages" "$ws"
          chmod -R u+w "$ws"
          mkdir -p "$ws/.cargo"
          printf '%s\n' '${cargoConfig}' > "$ws/.cargo/config.toml"
          cargo-deny --manifest-path "$ws/Cargo.toml" \
            check             --config "${rustPackagesSrc}/deny.toml" bans licenses sources
          echo ok > "$out"
        '';

        # Real cargo-audit gate: vulnerability scan of each checked-in
        # context policy lock against the pinned advisory DB snapshot. The
        # filtered locks are audit-only projections; Cargo resolution still
        # uses the repository-root lock. Advisory ignores, when approved,
        # are read only from the matching protected context.
        rust-audit = pkgs.runCommand "d2b-rust-audit" {
          nativeBuildInputs = [ pkgs.cargo-audit pkgs.jq ];
        } ''
          export HOME="$TMPDIR"
          policy_root=${rustPackagesSrc}/packages/policy-inputs
          advisory_policy=$policy_root/advisory-policy.json
          run_audit() {
            local lock=$1 context_key=$2 advisory_id
            shift 2
            local -a ignores=()
            if [ -n "$context_key" ]; then
              while IFS= read -r advisory_id; do
                [ -n "$advisory_id" ] && ignores+=(--ignore "$advisory_id")
              done < <(
                jq -r \
                  --arg context_key "$context_key" \
                  '.contexts[$context_key].advisories[]?.id' \
                  "$advisory_policy"
              )
            fi
            echo "==> cargo audit ($context_key)"
            cargo-audit audit --file "$lock" \
              --db ${advisoryDbGit} --no-fetch \
              "''${ignores[@]}" "$@"
          }
          while IFS= read -r lock; do
            relative="''${lock#"$policy_root"/}"
            IFS=/ read -r system target context _projection _lock <<< "$relative"
            run_audit "$lock" "$system/$target/$context"
          done < <(
            find "$policy_root" -type f -path '*/policy/Cargo.lock' | LC_ALL=C sort
          )
          run_audit ${rustPackagesSrc}/packages/Cargo.guest.lock ""
          echo ok > $out
        '';

        guest-static-dependency-policy =
          pkgs.runCommand "d2b-guest-static-dependency-policy" { } ''
            lock=${./packages/Cargo.guest.lock}
            if grep -E 'name = "(cc|cmake|pkg-config|openssl|openssl-sys|native-tls|libsystemd|systemd)"' "$lock"; then
              echo "guest static lock contains a native-link/build-script dependency" >&2
              exit 1
            fi
            echo ok > "$out"
          '';

        guest-shell-runner-static-dependency-policy =
          pkgs.runCommand "d2b-guest-shell-runner-static-dependency-policy" { } ''
            lock=${./Cargo.lock}
            if grep -E 'name = "(openssl|openssl-sys|native-tls|libsystemd|systemd|pam-sys|dlopen2)"' "$lock"; then
              echo "guest shell runner lock contains a forbidden dynamic/PAM/systemd dependency" >&2
              exit 1
            fi
            if ! grep -A6 'name = "motd"' "$lock" | grep -F 'version = "0.2.2"' >/dev/null; then
              echo "guest shell runner lock must pin the expected PAM-free motd dependency posture" >&2
              exit 1
            fi
            echo ok > "$out"
          '';

        harness-ubuntu-skeleton = (import ./harness/ubuntu/default.nix) {
          pkgs = nixpkgsFor.${system};
        };

        # Template eval-check: override the three sentinel-gated
        # fields (TODOs 2 + 3) so the assertion block passes. The
        # template module itself is imported unchanged so any
        # regression in the sentinel logic still surfaces here.
        eval-template = mkCheck "eval-template" (mkEval [
          (import ./templates/default/configuration.nix)
          ({ lib, ... }: {
            # Minimal NixOS baseline the template intentionally
            # omits (TODO 1 - hardware-configuration). Without this
            # the eval would fail on `fileSystems."/"`.
            boot.loader.systemd-boot.enable = lib.mkForce false;
            boot.loader.grub.enable = false;
            boot.initrd.includeDefaultModules = false;
            fileSystems."/" = {
              device = "tmpfs";
              fsType = "tmpfs";
            };
            environment.etc."machine-id".text =
              "00000000000000000000000000000000";

            # Sentinel overrides - these are the three fields gated
            # by the template's assertion block. Each `mkForce`
            # replaces a sentinel with a valid stand-in so the
            # assertions pass and the rest of the module eval runs.
            networking.hostName = lib.mkForce "check-template";
            d2b.site.launcherUsers = lib.mkForce [ "check-user" ];
            d2b.site.userAuthorizedKeys = lib.mkForce [
              "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBcheckcheckcheckcheckcheckcheckcheckchecky check@template-check"
            ];

            # The launcherUsers principal must be a real user.
            users.users.check-user = {
              isNormalUser = true;
              uid = 1100;
            };
          })
        ]);
      } // nixpkgs.lib.optionalAttrs (system == "x86_64-linux") {
        # graphics-workstation transitively depends on x86_64-only
        # packages (spectrum-ch, crosvm-patched, vhost-device-sound)
        # and the framework's `checkVmPlatform` gate refuses to
        # evaluate a graphics-enabled VM on a non-x86_64 host. Gate
        # the check on `system == "x86_64-linux"` so aarch64-linux
        # `nix flake check` stays green.
        eval-graphics = mkCheck "eval-graphics"
          (mkEval [ (import ./examples/graphics-workstation/configuration.nix) ]);
      });

      lib = nixpkgs.lib.makeExtensible (_: {
        evalFixture = system: self.checks.${system}.eval-fixture-contracts.fixtureData;
        buildProviderElfShim = providerElfShim;
      });

    };
}
