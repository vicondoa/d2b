{
  description = "Opinionated NixOS desktop microVM workspaces";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # `microvm` flake input DROPPED per ADR 0018.
    # The d2b NixOS substrate owns its per-VM evaluator via
    # `nixos-modules/vm-evaluator.nix` + `nixos-modules/vm-options.nix`.
    # Runner argv generation lives in the Rust crate
    # `packages/d2b-host/src/*_argv.rs` (broker-side).

    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, home-manager, ... }@inputs:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      nixpkgsFor = forAllSystems (system: import nixpkgs { inherit system; });
      providerElfShim = import ./nix/provider-elf-shim.nix;
      mkGuestRustPackagesSrc = pkgs:
        pkgs.runCommand "d2b-guest-rust-src" { } ''
          mkdir -p $out/packages
          cp -r ${./packages/d2b-realm-core} $out/packages/d2b-realm-core
          cp -r ${./packages/d2b-core} $out/packages/d2b-core
          cp -r ${./packages/d2b-contracts} $out/packages/d2b-contracts
          cp -r ${./packages/d2b-guestd} $out/packages/d2b-guestd
          cp -r ${./packages/d2b-exec-runner} $out/packages/d2b-exec-runner
          cp -r ${./packages/d2b-sk-frontend} $out/packages/d2b-sk-frontend
          cp ${./packages/Cargo.guest.lock} $out/packages/Cargo.lock
          chmod -R u+w $out/packages/d2b-core
          chmod -R u+w $out/packages/d2b-realm-core
          cp ${./tests/fixtures/guest-rust-workspace/d2b-realm-core.Cargo.toml} \
            $out/packages/d2b-realm-core/Cargo.toml
          cp ${./tests/fixtures/guest-rust-workspace/d2b-core.Cargo.toml} \
            $out/packages/d2b-core/Cargo.toml
          cp ${./tests/fixtures/guest-rust-workspace/Cargo.toml} \
            $out/packages/Cargo.toml
        '';
      # The Nix-unit corpus is shared by the topical flake checks, the
      # per-file nix-eval-jobs surface, and the locked inventory. Keep the
      # evaluator context in one constructor so those surfaces cannot drift.
      nixUnitCorpusFor = system:
        let
          pkgs = nixpkgsFor.${system};
          d2bModule = import ./nixos-modules { inherit inputs; };
          mkEval = modules: nixpkgs.lib.nixosSystem {
            inherit system;
            modules = [
              d2bModule
              ({ lib, ... }: {
                d2b.site.usePrebuiltHostTools =
                  lib.mkDefault (system == "x86_64-linux");
              })
            ] ++ modules;
          };
        in
        import ./tests/unit/nix/eval-jobs.nix {
          lib = pkgs.lib;
          inherit pkgs system;
          flakeRoot = ./.;
          d2bLib = import ./nixos-modules/lib.nix { lib = pkgs.lib; };
          inherit mkEval;
          nixpkgsFlake = nixpkgs;
          inherit d2bModule;
        };
      nixUnitShardCaseFiles = {
        nix-unit-daemon = [
          "activation-runtime-tmpfiles.nix"
          "broker-bundle-path.nix"
          "broker-caps.nix"
          "broker-service-posture.nix"
          "broker-socket-activation.nix"
          "bundle-artifacts-compiler.nix"
          "bundle-artifacts-digest.nix"
          "bundle-artifacts-envelope.nix"
          "daemon-autostart.nix"
          "daemon-default-compat.nix"
          "gateway-vm.nix"
          "d2bd-startup-smoke.nix"
        ];
        nix-unit-guest = [
          "guest-config-containment.nix"
          "guest-control-auth.nix"
          "guest-control-vsock.nix"
          "guest-exec-policy.nix"
          "guest-shell-policy.nix"
        ];
        nix-unit-misc = [
          "assertions.nix"
          "autostart-wiring.nix"
          "examples-with-observability.nix"
          "ifname-nix-rust-parity.nix"
          "observability.nix"
          "observability-guest.nix"
          "observability-host-collector.nix"
          "observability-host-collector-extra.nix"
          "observability-host-collector-otlp.nix"
          "observability-host-collector-processor-split.nix"
          "observability-host-collector-identity.nix"
          "observability-host-collector-umask.nix"
          "observability-host-collector-flags.nix"
          "provider-catalog.nix"
          "provider-elf-shim.nix"
          "provider-projection-exportability.nix"
          "provider-projection-fields.nix"
          "readiness-waves.nix"
          "resource-sharing.nix"
          "resources-bundle-telemetry.nix"
          "restart-policy.nix"
          "test-infrastructure.nix"
          "usb-security-key.nix"
          "vm-eval-overlays.nix"
        ];
        nix-unit-network = [
          "bridge-ipv6-boot-sysctl.nix"
          "generation-cleanup-absent-network.nix"
          "index.nix"
          "multi-env-daemon-backed.nix"
          "net-vm-network.nix"
          "realm-workloads.nix"
          "realms.nix"
          "usbip-gating.nix"
        ];
        nix-unit-runtime = [
          "clipboard.nix"
          "external-vm-kind.nix"
          "niri-vm-borders.nix"
          "requested-vm-config.nix"
          "security-key-gating.nix"
          "video-contract.nix"
        ];
        nix-unit-state = [
          "per-vm-state-ownership.nix"
          "principal-uid-collision.nix"
          "store-overlay-emit.nix"
          "umask-roundtrip.nix"
          "volume-mounts.nix"
        ];
      };
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
      #   apps.<sys>           - the `d2b` CLI as a runnable app
      #   templates.default    - `nix flake init -t github:vicondoa/d2b`
      #   checks.<sys>         - flake-eval CI gates
      #   lib                  - re-exported helpers (subnetIp, mkMac, …)
      #   overlays.default     - adds vhostDeviceSound, crosvmPatched, …
      nixosModules.default = import ./nixos-modules { inherit inputs; };

      # Developer shell: everything the Layer-1 gates need, in one place.
      #
      # Without this each gate script provisions its own toolchain, which is
      # why tests/test-rust.sh, tests/test-policy.sh and
      # tests/tools/assert-pinned-tests.sh each carry their own nix-shell
      # re-entry and rustup bootstrap. Enter this shell and those paths are
      # skipped entirely, because the tools they look for are already present.
      #
      # rustup rather than pkgs.rustc: packages/rust-toolchain.toml pins a
      # version nixpkgs does not carry (the pin is 1.97.0; this nixpkgs has
      # 1.95.0), and rustup reads that file itself. Once the nixpkgs input
      # advances far enough to supply the pinned release, rustup can be dropped
      # for pkgs.rustc/pkgs.cargo and the pin will be served natively.
      devShells = forAllSystems (system: let
        pkgs = nixpkgsFor.${system};
      in {
        default = pkgs.mkShell {
          name = "d2b-dev";
          packages = with pkgs; [
            # Toolchain. rustup resolves packages/rust-toolchain.toml.
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
            export SCCACHE_DIR="''${SCCACHE_DIR:-$HOME/.cache/d2b-sccache}"
            echo "d2b dev shell: rust $(sed -n 's/.*channel = "\(.*\)".*/\1/p' packages/rust-toolchain.toml) via rustup, sccache at $SCCACHE_DIR"
          '';
        };
        # Focused shell for the evaluation-only Nix-unit runner. Keeping this
        # output separate lets the target acquire only its locked external
        # tools instead of entering the full Rust development shell.
        nix-unit = pkgs.mkShellNoCC {
          name = "d2b-nix-unit";
          packages = with pkgs; [
            nix-eval-jobs
            jq
          ];
        };
      });

      packages = forAllSystems (system: let
        pkgs = nixpkgsFor.${system};
        rustPackagesSrc = pkgs.runCommand "d2b-rust-src" { } ''
          mkdir -p $out/packages
          cp -r ${./packages}/. $out/packages/
        '';
        rustWorkspace = args: pkgs.rustPlatform.buildRustPackage ({
          pname = "d2b-rust-workspace";
          version = "0.0.0-bootstrap";
          src = rustPackagesSrc;
          sourceRoot = "d2b-rust-src/packages";
          cargoLock = {
            lockFile = ./packages/Cargo.lock;
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
            src = ./packages/d2b-guest-shell-runner;
            cargoLock = {
              lockFile = ./packages/d2b-guest-shell-runner/Cargo.lock;
            };
            cargoBuildFlags = [ "--features" "real-libshpool" ];
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
          pname = "d2b-clipd";
          cargoBuildFlags = [ "--package" "d2b-clipd" "--bin" "d2b-clipd" ];
          doCheck = false;
        };
        d2b-wayland-proxy = rustWorkspace {
          pname = "d2b-wayland-proxy";
          cargoBuildFlags = [ "--package" "d2b-wayland-proxy" "--bin" "d2b-wayland-proxy" ];
          doCheck = false;
          meta.mainProgram = "d2b-wayland-proxy";
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

        signoz = import ./pkgs/signoz { inherit pkgs; };
        signozOtelCollector = import ./pkgs/signoz-otel-collector { inherit pkgs; };
        signozSchemaMigrator = import ./pkgs/signoz-schema-migrator { inherit pkgs; };
      });

      apps = forAllSystems (system: { });

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
              value = import (testDir + "/${file}") { inherit pkgs self; };
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
      # `tests/static.sh` examples-iteration step exercises it.
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
        # Compose a sandbox src that holds packages/ plus those fixture
        # trees so the cargo workspace never reads outside its packaged
        # source in the Nix sandbox. Operators running cargo OUTSIDE
        # the sandbox use the raw ./packages tree and the same relative
        # paths still resolve against the checkout.
        rustPackagesSrc = pkgs.runCommand "d2b-rust-src" { } ''
          mkdir -p $out/packages
          cp -r ${./packages}/. $out/packages/
          mkdir -p $out/tests
          cp -r ${./tests/golden} $out/tests/golden
          cp -r ${./tests/fixtures} $out/tests/fixtures
        '';
        guestRustPackagesSrc = mkGuestRustPackagesSrc pkgs;
        rustWorkspace = args: pkgs.rustPlatform.buildRustPackage ({
          pname = "d2b-rust-workspace";
          version = "0.0.0-bootstrap";
          src = rustPackagesSrc;
          sourceRoot = "d2b-rust-src/packages";
          cargoLock = {
            lockFile = ./packages/Cargo.lock;
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
          (builtins.fromTOML (builtins.readFile ./packages/rust-toolchain.toml)).toolchain.channel;
        brokerManifestToml = builtins.fromTOML (builtins.readFile ./packages/d2b-priv-broker/Cargo.toml);
        mainManifestToml = builtins.fromTOML (builtins.readFile ./packages/Cargo.toml);
        assertRustToolchain = ''
          rustc --version | grep -F "${rustToolchainChannel}"
        '';
        assertRustSupplyChainInputs = ''
          test -f ${rustPackagesSrc}/packages/Cargo.lock
          test -f ${rustPackagesSrc}/packages/Cargo.guest.lock
          test -f ${rustPackagesSrc}/packages/deny.toml
          test -f ${rustPackagesSrc}/packages/d2b-priv-broker/Cargo.lock
          test -f ${rustPackagesSrc}/packages/d2b-priv-broker/deny.toml
          test -f ${rustPackagesSrc}/packages/d2b-guest-shell-runner/Cargo.lock
          test -f ${rustPackagesSrc}/packages/d2b-guest-shell-runner/deny.toml
          printf '%s\n' '${builtins.toJSON mainManifestToml.workspace.members}' >/dev/null
          printf '%s\n' '${brokerManifestToml.package.name}' >/dev/null
          printf '%s\n' '${builtins.toJSON brokerManifestToml.workspace}' >/dev/null
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

        nixUnitCaseFileNames = nixUnitCorpus.caseFileNames;
        nixUnitShardFiles = pkgs.lib.concatLists (pkgs.lib.attrValues nixUnitShardCaseFiles);
        nixUnitShardMissingFiles =
          pkgs.lib.filter (n: !(builtins.elem n nixUnitShardFiles)) nixUnitCaseFileNames;
        nixUnitShardUnknownFiles =
          pkgs.lib.filter (n: !(builtins.elem n nixUnitCaseFileNames)) nixUnitShardFiles;
        nixUnitShardDuplicateFiles =
          let
            count = needle: pkgs.lib.length (pkgs.lib.filter (n: n == needle) nixUnitShardFiles);
          in
          pkgs.lib.filter (n: count n > 1) (pkgs.lib.unique nixUnitShardFiles);
        nixUnitShardCoverageOk =
          nixUnitShardMissingFiles == [ ]
          && nixUnitShardUnknownFiles == [ ]
          && nixUnitShardDuplicateFiles == [ ];
        nixUnitShardCoverageReport = builtins.toJSON {
          missing = nixUnitShardMissingFiles;
          unknown = nixUnitShardUnknownFiles;
          duplicate = nixUnitShardDuplicateFiles;
        };
        nixUnitCorpus = nixUnitCorpusFor system;
        nixUnitAggregateCheck = nixUnitCorpus.mkAggregateCheck;
        nixUnitShardChecks =
          pkgs.lib.mapAttrs nixUnitAggregateCheck nixUnitShardCaseFiles;

        # Fail-closed case-PRESENCE gate (mirrors tests/tools/assert-pinned-tests.sh
        # for the Rust layer): every pinned case name MUST still exist in the
        # corpus, so a retired bash gate's nix-unit successor can't silently
        # vanish. Pins are system-aware - `common.txt` holds the all-systems
        # cases; `<system>.txt` holds extra (e.g. x86-only graphics) cases.
        # Regenerate with `make nix-unit-pin` after adding/removing cases.
        #
        # common.txt is REQUIRED and must be non-empty: deleting the pin file
        # itself (along with case files) must fail closed, not silently make
        # the pin set empty (panel W2 finding). The PER-SYSTEM file is also
        # REQUIRED TO EXIST for the current system, but may be empty - a
        # system with no extra (e.g. graphics) cases still commits a
        # header-only file, so deleting a non-empty per-system pin file
        # (e.g. x86_64-linux.txt with its 42 graphics pins) also fails closed
        # (panel W2 re-review finding). The set of supported systems is the
        # flake's own `systems`, not the currently-evaluated case set (which
        # could be deleted in the same diff).
        nixUnitCorpusCaseNames = nixUnitCorpus.caseNames;
        pinNames = path: pkgs.lib.filter (n: n != "" && !(pkgs.lib.hasPrefix "#" n))
          (pkgs.lib.splitString "\n" (builtins.readFile path));
        readPinsRequiredNonEmpty = path:
          if !(builtins.pathExists path) then
            throw "nix-unit: required pin file ${toString path} is missing - run `make nix-unit-pin`"
          else
            let names = pinNames path;
            in if names == [ ]
            then throw "nix-unit: required pin file ${toString path} has no pinned cases - the corpus would be unguarded; run `make nix-unit-pin`"
            else names;
        readPinsRequiredExist = path:
          # The file MUST exist (so deleting it fails closed) but MAY be empty
          # for a system with no system-specific cases (e.g. aarch64 has no
          # x86-only graphics cases).
          if !(builtins.pathExists path) then
            throw "nix-unit: required per-system pin file ${toString path} is missing - every supported system commits one (header-only is fine); run `make nix-unit-pin`"
          else pinNames path;
        nixUnitPinned =
          (readPinsRequiredNonEmpty ./tests/unit/nix/pinned/common.txt)
          ++ (readPinsRequiredExist (./tests/unit/nix/pinned + "/${system}.txt"));
        nixUnitMissingPins =
          pkgs.lib.filter (n: !(builtins.elem n nixUnitCorpusCaseNames)) nixUnitPinned;
        nixUnitMissingReport = pkgs.lib.concatMapStringsSep "\n"
          (n: "MISSING PINNED CASE: ${n} (a pinned nix-unit case was deleted - restore it or run `make nix-unit-pin`)")
          nixUnitMissingPins;
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

        # W2: nix-unit value/throw assertions migrated from the group-D/E
        # eval-gate bash scripts.
        #
        # CRITICAL: failures THROW at EVALUATION time, not just at build time.
        # tests/static.sh + static-fast.sh run `nix flake check --no-build
        # --all-systems`, which evaluates every check's derivation but does
        # NOT build it. A failing runCommand would evaluate to a valid
        # (unbuilt) derivation and slip through fail-OPEN (panel W2 finding).
        # Throwing here forces the gate to fail during `--no-build`
        # evaluation, on BOTH systems (aarch64 included on an x86 runner).
        nix-unit =
          if !nixUnitShardCoverageOk || nixUnitMissingPins != [ ] then
            throw ''
              nix-unit presence gate FAILED (${toString (pkgs.lib.length nixUnitMissingPins)} pinned cases missing) for ${system}:
              shardCoverage=${nixUnitShardCoverageReport}${pkgs.lib.optionalString (nixUnitMissingPins != [ ]) "\n${nixUnitMissingReport}"}
            ''
          else
            pkgs.runCommand "d2b-nix-unit" { } ''
              echo "nix-unit: ${toString (pkgs.lib.length nixUnitCorpusCaseNames)} pinned case names present; ${toString (pkgs.lib.length (pkgs.lib.attrNames nixUnitShardCaseFiles))} shards cover ${toString (pkgs.lib.length nixUnitCaseFileNames)} case files"
              mkdir -p "$out"
              echo ok > "$out/nix-unit"
            '';

        # W2: the "module callsites use the shared volume helpers" grep
        # checks from volume-mounts-eval.sh - a hermetic source-wiring
        # invariant (the nix-unit value cases assert the helpers; this
        # asserts the production modules actually call them).
        module-helper-wiring = pkgs.runCommand "d2b-module-helper-wiring" { } ''
          set -e
          grep -Fq 'serial = d2bLib.volumeSerial volume;' ${./nixos-modules/processes-json.nix} \
            || { echo "processes-json.nix must use shared volumeSerial helper" >&2; exit 1; }
          grep -Fq 'd2bLib.volumeFileSystem volume' ${./nixos-modules/vm-guest-base.nix} \
            || { echo "vm-guest-base.nix must use shared volumeFileSystem helper" >&2; exit 1; }
          grep -Fq 'builtins.filter d2bLib.volumeDiskInitEligible microvm.volumes' ${./nixos-modules/processes-json.nix} \
            || { echo "processes-json.nix must gate DiskInit with shared eligibility helper" >&2; exit 1; }
          mkdir -p "$out"
          echo ok > "$out/module-helper-wiring"
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
          # Keep fixture-dependent contract crates out of generic sandbox
          # workspace tests. fixture-smoke only renders their input artifacts;
          # it does not execute these tests. Full D2B_FIXTURES delivery to the
          # sandbox/CI is tracked separately.
          cargoTestFlags = [
            "--workspace"
            "--exclude"
            "d2b-contract-tests"
          ];
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

        guest-static-elf = pkgs.runCommand "d2b-guest-static-elf" {
          nativeBuildInputs = [ pkgs.pkgsStatic.binutils ];
        } ''
          readelf=${pkgs.pkgsStatic.binutils.bintools}/bin/readelf
          for bin in \
            ${self.packages.${system}.d2b-guestd-static}/bin/d2b-guestd \
            ${self.packages.${system}.d2b-exec-runner-static}/bin/d2b-exec-runner \
            ${self.packages.${system}.d2b-sk-frontend-static}/bin/d2b-sk-frontend \
            ${self.packages.${system}.d2b-guest-shell-runner-static}/bin/d2b-guest-shell-runner
          do
            test -x "$bin"
            name="$(basename "$bin")"
            "$readelf" -h "$bin" >/dev/null
            "$readelf" -l "$bin" > "$TMPDIR/$name.program-headers"
            if grep -q 'Requesting program interpreter' "$TMPDIR/$name.program-headers"; then
              echo "$bin: unexpected ELF interpreter" >&2
              cat "$TMPDIR/$name.program-headers" >&2
              exit 1
            fi
            if "$readelf" -d "$bin" > "$TMPDIR/$name.dynamic" 2> "$TMPDIR/$name.dynamic.err"; then
              if grep -q '(NEEDED)' "$TMPDIR/$name.dynamic"; then
                echo "$bin: unexpected dynamic dependency" >&2
                cat "$TMPDIR/$name.dynamic" >&2
                exit 1
              fi
            elif ! grep -qi 'no dynamic section' "$TMPDIR/$name.dynamic.err"; then
              echo "$bin: readelf -d failed unexpectedly" >&2
              cat "$TMPDIR/$name.dynamic.err" >&2
              exit 1
            fi
          done
          mkdir -p "$out"
          echo ok > "$out/guest-static-elf"
        '';

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

        guest-exec-policy = let
          evidence = import ./tests/unit/nix/eval-cases/guest-exec-policy-eval.nix {
            inherit system pkgs;
            flake = self;
            scenario = "enabled";
          };
        in pkgs.runCommand "d2b-guest-exec-policy" { } ''
          mkdir -p "$out"
          printf '%s\n' '${evidence}' > "$out/guest-exec-policy.json"
        '';

        guest-control-vsock = let
          evidence = import ./tests/unit/nix/eval-cases/guest-control-vsock-eval.nix {
            inherit system pkgs;
            flake = self;
            scenario = "base";
          };
        in pkgs.runCommand "d2b-guest-control-vsock" { } ''
          mkdir -p "$out"
          printf '%s\n' '${evidence}' > "$out/guest-control-vsock.json"
        '';

        # Real cargo-deny gate: bans, licenses, and sources for both
        # the main workspace and the broker workspace.  Advisory
        # checks are handled by rust-audit below (cargo-deny requires
        # a fetchable URL for the advisory DB which is incompatible
        # with the Nix sandbox's no-network constraint).
        #
        # cargo-deny shells out to `cargo metadata`, so we vendor
        # the crate registry and override the sccache wrapper that
        # the repo-local .cargo/config.toml enables.
        rust-deny = let
          mainVendor = pkgs.rustPlatform.importCargoLock {
            lockFile = ./packages/Cargo.lock;
            outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
          };
          brokerVendor = pkgs.rustPlatform.importCargoLock {
            lockFile = ./packages/d2b-priv-broker/Cargo.lock;
          };
          guestShellRunnerVendor = pkgs.rustPlatform.importCargoLock {
            lockFile = ./packages/d2b-guest-shell-runner/Cargo.lock;
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
            cp -r "$src/packages" "$ws"
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
            "${rustPackagesSrc}/packages/deny.toml"

          run_deny "broker" \
            "${rustPackagesSrc}" \
            "d2b-priv-broker/Cargo.toml" \
            '${cargoConfig brokerVendor}' \
            "${rustPackagesSrc}/packages/d2b-priv-broker/deny.toml"

          run_deny "guest-shell-runner" \
            "${rustPackagesSrc}" \
            "d2b-guest-shell-runner/Cargo.toml" \
            '${cargoConfig guestShellRunnerVendor}' \
            "${rustPackagesSrc}/packages/d2b-guest-shell-runner/deny.toml"

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
            check --config "${rustPackagesSrc}/packages/deny.toml" bans licenses sources
          echo ok > "$out"
        '';

        # Real cargo-audit gate: vulnerability scan of every committed lockfile
        # against the pinned advisory DB snapshot.  Runs offline via
        # --no-fetch with the bundled git-repo copy of the RustSec DB.
        rust-audit = pkgs.runCommand "d2b-rust-audit" {
          nativeBuildInputs = [ pkgs.cargo-audit ];
        } ''
          export HOME="$TMPDIR"
          run_audit() {
            local lock=$1
            shift
            echo "==> cargo audit ($(basename "$(dirname "$lock")"))"
            cargo-audit audit --file "$lock" \
              --db ${advisoryDbGit} --no-fetch "$@"
          }
          # Build-time wayland-scanner pulls quick-xml 0.39.4; runtime users
          # were updated away from vulnerable 0.37.x. Remove once
          # wayland-scanner publishes a release on quick-xml >= 0.41.
          run_audit ${rustPackagesSrc}/packages/Cargo.lock \
            --ignore RUSTSEC-2026-0194 \
            --ignore RUSTSEC-2026-0195
          run_audit ${rustPackagesSrc}/packages/Cargo.guest.lock
          run_audit ${rustPackagesSrc}/packages/d2b-priv-broker/Cargo.lock
          # libshpool 0.11.0 pulls notify 7 -> notify-types -> instant 0.1.13.
          # Track that feasibility-spike warning explicitly while the helper
          # evaluates the pinned shpool dependency.
          run_audit ${rustPackagesSrc}/packages/d2b-guest-shell-runner/Cargo.lock \
            --ignore RUSTSEC-2024-0384
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
            lock=${./packages/d2b-guest-shell-runner/Cargo.lock}
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
      } // nixUnitShardChecks // nixpkgs.lib.optionalAttrs (system == "x86_64-linux") {
        # graphics-workstation transitively depends on x86_64-only
        # packages (spectrum-ch, crosvm-patched, vhost-device-sound)
        # and the framework's `checkVmPlatform` gate refuses to
        # evaluate a graphics-enabled VM on a non-x86_64 host. Gate
        # the check on `system == "x86_64-linux"` so aarch64-linux
        # `nix flake check` stays green.
        eval-graphics = mkCheck "eval-graphics"
          (mkEval [ (import ./examples/graphics-workstation/configuration.nix) ]);
      });

      # The local nix-eval-jobs surface is the middle partition: one aggregate
      # attr per case file. The constructor lives in eval-jobs.nix and is also
      # used by the seven topical flake checks above.
      nixUnitJobs = forAllSystems (system:
        let
          nixUnitCorpus = nixUnitCorpusFor system;
        in
        nixUnitCorpus.fileJobs // {
          nix-unit = self.checks.${system}.nix-unit;
        }
      );

      # Evaluate file jobs in existing topical shard processes. This keeps
      # coverage and the locked file-job inventory unchanged while preventing
      # one nix-eval-jobs worker from retaining every large scenario graph for
      # the entire corpus.
      nixUnitJobShards = forAllSystems (system:
        let
          pkgs = nixpkgsFor.${system};
          nixUnitCorpus = nixUnitCorpusFor system;
          jobsFor = files:
            pkgs.lib.filterAttrs
              (jobName: _:
                builtins.elem
                  "${pkgs.lib.removePrefix "case-" jobName}.nix"
                  files)
              nixUnitCorpus.fileJobs;
        in
        (pkgs.lib.mapAttrs (_: jobsFor) nixUnitShardCaseFiles) // {
          integrity = {
            nix-unit = self.checks.${system}.nix-unit;
          };
        }
      );

      # One locked, evaluation-only inventory keeps both exact source case
      # names and the per-file job names together. Attr-name discovery does
      # not force any case expression.
      nixUnitInventory = forAllSystems (system:
        let
          nixUnitCorpus = nixUnitCorpusFor system;
        in
        {
          caseNames = builtins.sort builtins.lessThan nixUnitCorpus.caseNames;
          jobNames =
            builtins.sort builtins.lessThan (nixUnitCorpus.jobNames ++ [ "nix-unit" ]);
        }
      );

      lib = nixpkgs.lib.makeExtensible (_: {
        evalFixture = system: self.checks.${system}.eval-fixture-contracts.fixtureData;
        buildProviderElfShim = providerElfShim;
      });

      overlays.default = _final: _prev: { };
    };
}
