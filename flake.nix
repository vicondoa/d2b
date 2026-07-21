{
  description = "Opinionated NixOS desktop microVM workspaces";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # `microvm` flake input DROPPED per ADR 0018.
    # The d2b NixOS substrate owns its realm workload evaluator via
    # `nixos-modules/vm-evaluator.nix`.
    # Runner argv generation lives in the Rust crate
    # `packages/d2b-host/src/*_argv.rs` (broker-side).

    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, home-manager, rust-overlay, ... }@inputs:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      nixpkgsFor = forAllSystems (system: import nixpkgs { inherit system; });
      deliveryPkgsFor = forAllSystems (system: import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      });
      rawShippedRustPackages = [
        {
          cargoPackage = "d2b";
          targetKind = "binary";
          flakePackage = null;
        }
        {
          cargoPackage = "d2b-clipd";
          targetKind = "binary";
          flakePackage = {
            output = "d2b-clipd";
            buildKind = "workspace";
            binary = "d2b-clipd";
            mainProgram = null;
          };
        }
        {
          cargoPackage = "d2b-exec-runner";
          targetKind = "binary";
          flakePackage = {
            output = "d2b-exec-runner-static";
            buildKind = "guestStatic";
            binary = "d2b-exec-runner";
            mainProgram = null;
          };
        }
        {
          cargoPackage = "d2b-gateway";
          targetKind = "library";
          flakePackage = null;
        }
        {
          cargoPackage = "d2b-gateway-runtime";
          targetKind = "binary";
          flakePackage = null;
        }
        {
          cargoPackage = "d2b-guest-shell-runner";
          targetKind = "binary";
          flakePackage = {
            output = "d2b-guest-shell-runner-static";
            buildKind = "guestShellStatic";
            binary = "d2b-guest-shell-runner";
            mainProgram = null;
          };
        }
        {
          cargoPackage = "d2b-guestd";
          targetKind = "binary";
          flakePackage = {
            output = "d2b-guestd-static";
            buildKind = "guestStatic";
            binary = "d2b-guestd";
            mainProgram = null;
          };
        }
        {
          cargoPackage = "d2b-host";
          targetKind = "binary";
          flakePackage = null;
        }
        {
          cargoPackage = "d2b-host-activation-helper";
          targetKind = "binary";
          flakePackage = null;
        }
        {
          cargoPackage = "d2b-notify";
          targetKind = "binary";
          flakePackage = null;
        }
        {
          cargoPackage = "d2b-priv-broker";
          targetKind = "binary";
          flakePackage = null;
        }
        {
          cargoPackage = "d2b-provider-relay";
          targetKind = "binary";
          flakePackage = null;
        }
        {
          cargoPackage = "d2b-sk-frontend";
          targetKind = "binary";
          flakePackage = {
            output = "d2b-sk-frontend-static";
            buildKind = "guestStatic";
            binary = "d2b-sk-frontend";
            mainProgram = null;
          };
        }
        {
          cargoPackage = "d2b-unsafe-local-helper";
          targetKind = "binary";
          flakePackage = {
            output = "d2b-unsafe-local-helper";
            buildKind = "workspace";
            binary = "d2b-unsafe-local-helper";
            mainProgram = "d2b-unsafe-local-helper";
          };
        }
        {
          cargoPackage = "d2b-userd";
          targetKind = "binary";
          flakePackage = {
            output = "d2b-userd-static";
            buildKind = "guestStatic";
            binary = "d2b-userd";
            mainProgram = null;
          };
        }
        {
          cargoPackage = "d2b-wayland-proxy";
          targetKind = "binary";
          flakePackage = {
            output = "d2b-wayland-proxy";
            buildKind = "workspace";
            binary = "d2b-wayland-proxy";
            mainProgram = "d2b-wayland-proxy";
          };
        }
        {
          cargoPackage = "d2bd";
          targetKind = "binary";
          flakePackage = null;
        }
        {
          cargoPackage = "xtask";
          targetKind = "binary";
          flakePackage = {
            output = "d2b-delivery";
            buildKind = "deliveryWorkspace";
            binary = "xtask";
            mainProgram = "xtask";
          };
        }
      ];
      shippedRustPackages =
        let
          cargoPackages = map (entry: entry.cargoPackage) rawShippedRustPackages;
          flakePackages =
            map (entry: entry.flakePackage.output)
              (builtins.filter (entry: entry.flakePackage != null) rawShippedRustPackages);
          validEntry = entry:
            builtins.isString entry.cargoPackage
            && builtins.elem entry.targetKind [ "binary" "library" ]
            && (
              entry.flakePackage == null
              || (
                entry.targetKind == "binary"
                && builtins.isString entry.flakePackage.output
                && builtins.isString entry.flakePackage.binary
                && builtins.elem entry.flakePackage.buildKind [
                  "deliveryWorkspace"
                  "guestShellStatic"
                  "guestStatic"
                  "workspace"
                ]
              )
            );
        in
        assert builtins.all validEntry rawShippedRustPackages;
        assert cargoPackages == builtins.sort builtins.lessThan cargoPackages;
        assert builtins.length cargoPackages
          == builtins.length (nixpkgs.lib.unique cargoPackages);
        assert builtins.length flakePackages
          == builtins.length (nixpkgs.lib.unique flakePackages);
        rawShippedRustPackages;
    in
    {
      # The public surface area — populated incrementally by the
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
      #   packages.<sys>       — patched cloud-hypervisor, crosvm, etc.
      #   apps.<sys>           — the `d2b` CLI as a runnable app
      #   templates.default    — `nix flake init -t github:vicondoa/d2b`
      #   checks.<sys>         — flake-eval CI gates
      #   lib                  — re-exported helpers (subnetIp, mkMac, …)
      #   overlays.default     — adds vhostDeviceSound, crosvmPatched, …
      nixosModules.default = import ./nixos-modules { inherit inputs; };

      packages = forAllSystems (system: let
        pkgs = nixpkgsFor.${system};
        deliveryTools = import ./pkgs/delivery-tools.nix {
          pkgs = deliveryPkgsFor.${system};
        };
        rustPackagesSrc = pkgs.runCommand "d2b-rust-src" { } ''
          mkdir -p $out/packages
          cp -r ${./packages}/. $out/packages/
        '';
        workspaceVersion =
          (builtins.fromTOML (builtins.readFile ./packages/Cargo.toml))
            .workspace.package.version;
        workspaceCargoLock = {
          lockFile = ./packages/Cargo.lock;
          outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
        };
        rustWorkspaceWith = rustPlatform: args: rustPlatform.buildRustPackage ({
          pname = "d2b-rust-workspace";
          version = workspaceVersion;
          src = rustPackagesSrc;
          sourceRoot = "d2b-rust-src/packages";
          cargoLock = workspaceCargoLock;
          RUSTC_WRAPPER = "";
          SCCACHE_DIR = "";
        } // args);
        rustWorkspace = rustWorkspaceWith pkgs.rustPlatform;
        deliveryRustWorkspace =
          rustWorkspaceWith deliveryTools.stableRustPlatform;
        deliveryRuntimeToolSpecs = [
          {
            name = "git";
            package = pkgs.git;
          }
          {
            name = "openssl";
            package = pkgs.openssl;
          }
          {
            name = "shellcheck";
            package = pkgs.shellcheck;
          }
          {
            name = "gh";
            package = deliveryTools.gh;
          }
          {
            name = "git-town";
            package = deliveryTools.gitTown;
          }
        ];
        deliveryRuntimePackages =
          map (tool: tool.package) deliveryRuntimeToolSpecs;
        deliveryRuntimeTools = map (tool: {
          inherit (tool) name;
          binPath = "${tool.package}/bin";
        }) deliveryRuntimeToolSpecs;
        guestStaticPackage = packageName: binName:
          pkgs.pkgsStatic.rustPlatform.buildRustPackage {
            pname = "${binName}-static";
            version = workspaceVersion;
            src = rustPackagesSrc;
            sourceRoot = "d2b-rust-src/packages";
            cargoLock = workspaceCargoLock;
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
        guestShellRunnerStatic = packageName: binName:
          pkgs.pkgsStatic.rustPlatform.buildRustPackage {
            pname = "${binName}-static";
            version = workspaceVersion;
            src = rustPackagesSrc;
            sourceRoot = "d2b-rust-src/packages";
            cargoLock = workspaceCargoLock;
            cargoBuildFlags = [
              "--package"
              packageName
              "--bin"
              binName
              "--features"
              "real-libshpool"
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
        shippedRustFlakePackages =
          builtins.filter (entry: entry.flakePackage != null) shippedRustPackages;
        mkShippedRustPackage = entry:
          let
            spec = entry.flakePackage;
            workspaceArgs = {
              pname = spec.output;
              cargoBuildFlags = [
                "--package"
                entry.cargoPackage
                "--bin"
                spec.binary
              ];
              doCheck = false;
            } // pkgs.lib.optionalAttrs (spec.mainProgram != null) {
              meta.mainProgram = spec.mainProgram;
            };
          in {
            name = spec.output;
            value =
              if spec.buildKind == "guestStatic" then
                guestStaticPackage entry.cargoPackage spec.binary
              else if spec.buildKind == "guestShellStatic" then
                guestShellRunnerStatic entry.cargoPackage spec.binary
              else if spec.buildKind == "workspace" then
                rustWorkspace workspaceArgs
              else if spec.buildKind == "deliveryWorkspace" then
                deliveryRustWorkspace (workspaceArgs // {
                  nativeBuildInputs = [ pkgs.makeWrapper pkgs.protobuf ];
                  postFixup = ''
                    wrapProgram "$out/bin/${spec.binary}" \
                      --prefix PATH : ${pkgs.lib.makeBinPath deliveryRuntimePackages}
                  '';
                  passthru = {
                    rustToolchainVersion = deliveryTools.rustStableVersion;
                    inherit deliveryRuntimeTools;
                  };
                })
              else
                throw "unsupported shipped Rust package build kind ${spec.buildKind}";
          };
        shippedRustPackageOutputs =
          builtins.listToAttrs (map mkShippedRustPackage shippedRustFlakePackages);
      in {
        manpages = pkgs.runCommand "d2b-manpages" { } ''
          install -Dm644 ${./docs/manpages/d2b.1} "$out/share/man/man1/d2b.1"
          ${pkgs.gzip}/bin/gzip -n -c ${./docs/manpages/d2b.1} > "$out/share/man/man1/d2b.1.gz"
        '';

        completions = pkgs.runCommand "d2b-completions" { } ''
          install -Dm644 ${./docs/completions/d2b.bash} "$out/share/bash-completion/completions/d2b"
          install -Dm644 ${./docs/completions/d2b.zsh}  "$out/share/zsh/site-functions/_d2b"
          install -Dm644 ${./docs/completions/d2b.fish} "$out/share/fish/vendor_completions.d/d2b.fish"
        '';
        git-town = deliveryTools.gitTown;
        gh = deliveryTools.gh;
        cargo-udeps-nightly = deliveryTools.cargoUdepsNightly;
        cargo-semver-checks = deliveryTools.cargoSemverChecks;

        signoz = import ./pkgs/signoz { inherit pkgs; };
        signozOtelCollector = import ./pkgs/signoz-otel-collector { inherit pkgs; };
        signozSchemaMigrator = import ./pkgs/signoz-schema-migrator { inherit pkgs; };
      } // shippedRustPackageOutputs);

      apps = forAllSystems (system: {
        delivery = {
          type = "app";
          program = "${self.packages.${system}.d2b-delivery}/bin/xtask";
        };
      });

      devShells = forAllSystems (system: let
        pkgs = deliveryPkgsFor.${system};
        deliveryTools = import ./pkgs/delivery-tools.nix { inherit pkgs; };
        shell = pkgs.mkShell {
          packages = [
            pkgs.cmake
            pkgs.git
            pkgs.jq
            pkgs.openssl
            pkgs.pkg-config
            pkgs.protobuf
            pkgs.sccache
            pkgs.shellcheck
            pkgs.stdenv.cc
            deliveryTools.stableRust
            deliveryTools.gh
            deliveryTools.gitTown
            deliveryTools.cargoUdepsNightly
            deliveryTools.cargoSemverChecks
          ];
        };
      in {
        default = shell;
        delivery = shell;
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
      # userland — e.g. proving a static d2b binary runs on stock Ubuntu.
      # It deliberately does NOT boot systemd for daemon/socket activation;
      # that is covered natively by
      # packages/d2b-priv-broker/tests/socket_activation.rs plus nix-unit.
      # See tests/integration/containers/README.md.
      #
      # Auto-discovered from tests/integration/containers/images/*.nix: each image module is
      # `{ pkgs, self, system }: <dockerTools-built OCI image>`, so adding a new
      # container test is one new image file + its tests/integration/containers/<name>.sh
      # runner — no edit here. x86_64-linux only (the project's CI runners +
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
      # is one new file — no edit here. x86_64-linux only: a runNixOSTest VM is
      # built + booted for the builder's own system, and the hosted CI runners
      # are x86_64 — aarch64 VM coverage needs an aarch64 builder.
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
        description = "Minimal d2b host scaffold — one env, one headless workload VM";
      };

      # Eval-only gates for the in-tree examples + template. The
      # `system.build.toplevel.drvPath` access is enough to force a
      # full module-system instantiation (option types, assertions,
      # CIDR validators, etc.) without actually realising the closure
      # — which is what we want from a `nix flake check` gate.
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
        deliveryTools = import ./pkgs/delivery-tools.nix {
          pkgs = deliveryPkgsFor.${system};
        };
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

          d2b.acceptDestructiveV2Cutover = true;

          d2b.realms.host = {
            path = "host";
            placement = "host-local";
            allowedUsers = [ "alice" ];
            policy.allowUnsafeLocal = true;
            providers.runtime = {
              type = "runtime";
              implementationId = "systemd-user";
            };
            providers.display = {
              type = "display";
              implementationId = "wayland";
            };
            network.ui.accentColor = "#cc3344";
            workloads.tools = {
              providerRefs.runtime = "runtime";
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
          d2b.realms.work = {
            path = "work";
            placement = "host-local";
            broker = {
              enable = true;
              hostMutation = true;
            };
            network = {
              mode = "declared";
              lanSubnet = "10.20.0.0/24";
              uplinkSubnet = "192.0.2.0/30";
            };
            allowedUsers = [ "alice" ];
            providers.runtime = {
              type = "runtime";
              implementationId = "cloud-hypervisor";
            };
            workloads.corp-runtime = {
              providerRefs.runtime = "runtime";
              launcher.enable = false;
              config = {
                networking.hostName = lib.mkDefault "corp-runtime";
                users.users.alice = {
                  isNormalUser = true;
                  uid = 1000;
                };
              };
            };
          };
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
        ];
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
          cp ${bundle.realmWorkloadsLauncherV2Json.path} $out/realm-workloads-launcher-v2.json
          cp ${bundle.unsafeLocalWorkloadsJson.path} $out/unsafe-local-workloads.json
          cp ${bundle.providerRegistryV2Json.path} $out/provider-registry-v2.json
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
        # so this is referenced only under that guard below (lazily — never
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
          d2b.acceptDestructiveV2Cutover = true;

          d2b.realms.work = {
            path = "work";
            placement = "host-local";
            broker = {
              enable = true;
              hostMutation = true;
            };
            network = {
              mode = "declared";
              lanSubnet = "10.20.0.0/24";
              uplinkSubnet = "192.0.2.0/30";
            };
            providers.runtime = {
              type = "runtime";
              implementationId = "cloud-hypervisor";
            };
            providers.devices = {
              type = "device";
              implementationId = "host-mediated";
            };
            providers.audio = {
              type = "audio";
              implementationId = "pipewire-vhost-user";
            };
            providers.display = {
              type = "display";
              implementationId = "wayland";
            };
            workloads.corp-full = {
              providerRefs = {
                runtime = "runtime";
                device = "devices";
                audio = "audio";
                display = "display";
              };
              tpm.enable = true;
              graphics = {
                enable = true;
                videoSidecar = true;
              };
              audio.enable = true;
              usbip.enable = true;
              display.wayland = true;
              config = {
                networking.hostName = lib.mkDefault "corp-full";
                users.users.alice = {
                  isNormalUser = true;
                  uid = 1000;
                };
              };
            };
            workloads.corp-render = {
              providerRefs = {
                runtime = "runtime";
                device = "devices";
                display = "display";
              };
              graphics = {
                enable = true;
                renderNodeOnly = true;
              };
              display.wayland = true;
              config = {
                networking.hostName = lib.mkDefault "corp-render";
                users.users.alice = {
                  isNormalUser = true;
                  uid = 1000;
                };
              };
            };
            workloads.corp-fido = {
              providerRefs = {
                runtime = "runtime";
                device = "devices";
              };
              securityKey.enable = true;
              config = {
                networking.hostName = lib.mkDefault "corp-fido";
                users.users.alice = {
                  isNormalUser = true;
                  uid = 1000;
                };
              };
            };
          };
        };
        fullEval = mkEval [
          fullConfigModule
          ({ lib, ... }: {
            # See smokeEval above: fixture-smoke-full is a rendered-contract
            # oracle, so it must consume source-built host tools.
            d2b.site.usePrebuiltHostTools = lib.mkForce false;
          })
        ];
        fullFixture = let
          bundle = fullEval.config.d2b._bundle;
          manifestPkg = fullEval.config.d2b._manifestPkg;
        in pkgs.runCommand "d2b-fixture-smoke-full" { } ''
          mkdir -p $out $out/closures
          cp ${bundle.privilegesJson.path} $out/privileges.json
          cp ${bundle.hostJson.path} $out/host.json
          cp ${bundle.processesJson.path} $out/processes.json
          cp ${bundle.storageJson.path} $out/storage.json
          cp ${bundle.syncJson.path} $out/sync.json
          cp ${bundle.allocatorJson.path} $out/allocator.json
          cp ${bundle.realmControllersJson.path} $out/realm-controllers.json
          cp ${bundle.realmIdentityJson.path} $out/realm-identity.json
          cp ${bundle.providerRegistryV2Json.path} $out/provider-registry-v2.json
          cp ${bundle.bundle.path} $out/bundle.json
          cp ${manifestPkg}/share/d2b/vms.json $out/manifest.json
          ${nixpkgs.lib.concatStringsSep "\n" (nixpkgs.lib.mapAttrsToList
            (vm: c: "cp ${c.path} $out/closures/${vm}.json")
            fullEval.config.d2b._bundle.closures)}
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
        workspaceVersion =
          (builtins.fromTOML (builtins.readFile ./packages/Cargo.toml))
            .workspace.package.version;
        workspaceCargoLock = {
          lockFile = ./packages/Cargo.lock;
          outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
        };
        rustWorkspace = args: pkgs.rustPlatform.buildRustPackage ({
          pname = "d2b-rust-workspace";
          version = workspaceVersion;
          src = rustPackagesSrc;
          sourceRoot = "d2b-rust-src/packages";
          cargoLock = workspaceCargoLock;
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
        mainManifestToml = builtins.fromTOML (builtins.readFile ./packages/Cargo.toml);
        assertRustToolchain = ''
          rustc --version | grep -F "${rustToolchainChannel}"
        '';
        assertRustSupplyChainInputs = ''
          test -f ${rustPackagesSrc}/packages/Cargo.lock
          test -f ${rustPackagesSrc}/packages/deny.toml
          test -f ${rustPackagesSrc}/packages/d2b-priv-broker/deny.toml
          test -f ${rustPackagesSrc}/packages/d2b-guest-shell-runner/deny.toml
          printf '%s\n' '${builtins.toJSON mainManifestToml.workspace.members}' >/dev/null
          test '${mainManifestToml.workspace.package.version}' = '2.0.0'
        '';
        workspaceVendor = pkgs.rustPlatform.importCargoLock workspaceCargoLock;
        workspaceVendorConfig = ''
          [source.crates-io]
          replace-with = "vendored-sources"
          [source."git+https://github.com/vicondoa/wl-proxy.git?rev=072945b59fef21a2a8166460454280d543f48772"]
          git = "https://github.com/vicondoa/wl-proxy.git"
          rev = "072945b59fef21a2a8166460454280d543f48772"
          replace-with = "vendored-sources"
          [source.vendored-sources]
          directory = "${workspaceVendor}"
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

        # --- W2 nix-unit layer -------------------------------------------
        # Hermetic pure-eval comparison runner over the tests/unit/nix
        # corpus ({ expr; expected; } / { expr; expectedError; } cases).
        # NO recursive-nix / IFD: each case is compared at flake-eval time
        # and the verdict baked into a tiny runCommand. The same corpus is
        # CLI-compatible with upstream `nix-unit` for local iteration.
        nixUnitShardCaseFiles = {
          nix-unit-daemon = [
            "activation-runtime-tmpfiles.nix"
            "broker-bundle-path.nix"
            "broker-caps.nix"
            "broker-service-posture.nix"
            "broker-socket-activation.nix"
            "bundle-artifacts.nix"
            "daemon-autostart.nix"
            "daemon-default-compat.nix"
            "d2bd-startup-smoke.nix"
            "provider-registry-v2.nix"
            "realm-allocator-emission.nix"
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
            "readiness-waves.nix"
            "restart-policy.nix"
            "usb-security-key.nix"
            "v2-identity.nix"
            "vm-eval-overlays.nix"
          ];
          nix-unit-network = [
            "bridge-ipv6-boot-sysctl.nix"
            "index.nix"
            "multi-env-daemon-backed.nix"
            "net-vm-network.nix"
            "platform-provider-mappings.nix"
            "realm-workloads.nix"
            "realms.nix"
            "usbip-gating.nix"
          ];
          nix-unit-runtime = [
            "clipboard.nix"
            "external-vm-kind.nix"
            "niri-vm-borders.nix"
            "realm-audio-resources.nix"
            "requested-vm-config.nix"
            "security-key-gating.nix"
            "unsafe-local-controller-allowlist.nix"
            "video-contract.nix"
          ];
          nix-unit-state = [
            "per-vm-state-ownership.nix"
            "principal-uid-collision.nix"
            "principal-workload-roles.nix"
            "store-overlay-emit.nix"
            "umask-roundtrip.nix"
            "volume-mounts.nix"
          ];
        };
        nixUnitCaseFileNames =
          pkgs.lib.filter (n: pkgs.lib.hasSuffix ".nix" n)
            (pkgs.lib.attrNames (builtins.readDir ./tests/unit/nix/cases));
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
        nixUnitCasesFor = caseFileNames: import ./tests/unit/nix {
          lib = pkgs.lib;
          inherit pkgs system;
          flakeRoot = ./.;
          d2bLib = import ./nixos-modules/lib.nix { lib = pkgs.lib; };
          inherit mkEval;
          # Direct-injection handles for tests/unit/nix/eval-cases/shared.nix (the
          # minimal lib.evalModules fast evaluator) — passing the nixpkgs
          # flake input + the d2b module set avoids a `getFlake ./.`
          # (which would resolve to a non-git store path inside the flake).
          nixpkgsFlake = nixpkgs;
          inherit d2bModule;
          inherit caseFileNames;
        };
        nixUnitCases = nixUnitCasesFor null;
        nixUnitEval = name: case:
          let
            r = builtins.tryEval (let v = case.expr; in builtins.deepSeq v v);
          in
          if case ? expectedError then
            # Bucket-B: the case must throw. `tryEval` cannot capture the
            # message, so message-substring matching is NOT supported here:
            # if an author sets `expectedError.msg` (expecting it enforced),
            # fail loudly rather than give false confidence. Message-sensitive
            # negative gates should assert over `config.assertions` data (see
            # guest-config-containment.nix) instead.
            if (builtins.isAttrs case.expectedError) && (case.expectedError != { }) then
              {
                inherit name;
                ok = false;
                detail = "expectedError must be `{ }` — this runner asserts only THAT the expr throws; tryEval cannot match a throw message. Move message-substring checks to config.assertions data.";
              }
            else
              {
                inherit name;
                ok = !r.success;
                detail =
                  if r.success
                  then "expected an error, but eval succeeded"
                  else "threw as expected";
              }
          else
            {
              inherit name;
              ok = r.success && r.value == case.expected;
              detail =
                if !r.success then "eval threw; expected a value"
                else "got=${builtins.toJSON r.value} expected=${builtins.toJSON case.expected}";
            };
        nixUnitResultsFor = cases: pkgs.lib.mapAttrsToList nixUnitEval cases;
        nixUnitShardCheck = checkName: caseFileNames:
          let
            cases = nixUnitCasesFor caseFileNames;
            results = nixUnitResultsFor cases;
            failures = pkgs.lib.filter (x: !x.ok) results;
            report = pkgs.lib.concatMapStringsSep "\n"
              (x: "FAIL ${x.name}: ${x.detail}") failures;
            total = pkgs.lib.length results;
          in
          if failures != [ ] then
            throw ''
              ${checkName} gate FAILED (${toString (pkgs.lib.length failures)}/${toString total} cases failed) for ${system}:
              ${report}
            ''
          else
            pkgs.runCommand "d2b-${checkName}" { } ''
              echo "${checkName}: ${toString total} cases passed"
              mkdir -p "$out"
              echo ok > "$out/${checkName}"
            '';
        nixUnitShardChecks =
          pkgs.lib.mapAttrs nixUnitShardCheck nixUnitShardCaseFiles;

        # Fail-closed case-PRESENCE gate (mirrors tests/tools/assert-pinned-tests.sh
        # for the Rust layer): every pinned case name MUST still exist in the
        # corpus, so a retired bash gate's nix-unit successor can't silently
        # vanish. Pins are system-aware — `common.txt` holds the all-systems
        # cases; `<system>.txt` holds extra (e.g. x86-only graphics) cases.
        # Regenerate with `make nix-unit-pin` after adding/removing cases.
        #
        # common.txt is REQUIRED and must be non-empty: deleting the pin file
        # itself (along with case files) must fail closed, not silently make
        # the pin set empty (panel W2 finding). The PER-SYSTEM file is also
        # REQUIRED TO EXIST for the current system, but may be empty — a
        # system with no extra (e.g. graphics) cases still commits a
        # header-only file, so deleting a non-empty per-system pin file
        # (e.g. x86_64-linux.txt with its 42 graphics pins) also fails closed
        # (panel W2 re-review finding). The set of supported systems is the
        # flake's own `systems`, not the currently-evaluated case set (which
        # could be deleted in the same diff).
        nixUnitCaseNames = pkgs.lib.attrNames nixUnitCases;
        pinNames = path: pkgs.lib.filter (n: n != "" && !(pkgs.lib.hasPrefix "#" n))
          (pkgs.lib.splitString "\n" (builtins.readFile path));
        readPinsRequiredNonEmpty = path:
          if !(builtins.pathExists path) then
            throw "nix-unit: required pin file ${toString path} is missing — run `make nix-unit-pin`"
          else
            let names = pinNames path;
            in if names == [ ]
            then throw "nix-unit: required pin file ${toString path} has no pinned cases — the corpus would be unguarded; run `make nix-unit-pin`"
            else names;
        readPinsRequiredExist = path:
          # The file MUST exist (so deleting it fails closed) but MAY be empty
          # for a system with no system-specific cases (e.g. aarch64 has no
          # x86-only graphics cases).
          if !(builtins.pathExists path) then
            throw "nix-unit: required per-system pin file ${toString path} is missing — every supported system commits one (header-only is fine); run `make nix-unit-pin`"
          else pinNames path;
        nixUnitPinned =
          (readPinsRequiredNonEmpty ./tests/unit/nix/pinned/common.txt)
          ++ (readPinsRequiredExist (./tests/unit/nix/pinned + "/${system}.txt"));
        nixUnitMissingPins =
          pkgs.lib.filter (n: !(builtins.elem n nixUnitCaseNames)) nixUnitPinned;
        nixUnitMissingReport = pkgs.lib.concatMapStringsSep "\n"
          (n: "MISSING PINNED CASE: ${n} (a pinned nix-unit case was deleted — restore it or run `make nix-unit-pin`)")
          nixUnitMissingPins;
      in {
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
              echo "nix-unit: ${toString (pkgs.lib.length nixUnitCaseNames)} pinned case names present; ${toString (pkgs.lib.length (pkgs.lib.attrNames nixUnitShardCaseFiles))} shards cover ${toString (pkgs.lib.length nixUnitCaseFileNames)} case files"
              mkdir -p "$out"
              echo ok > "$out/nix-unit"
            '';

        # W2: the "module callsites use the shared volume helpers" grep
        # checks from volume-mounts-eval.sh — a hermetic source-wiring
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
              d2b.realms.work.network.mtu = lib.mkForce 1400;
              d2b.realms.work.network.mssClamp = lib.mkForce true;
              d2b.realms.work.network.lan.allowEastWest = lib.mkForce true;
            })
          ]);

        eval-with-observability =
          let
            cfg = mkEval [ (import ./examples/with-observability/configuration.nix) ];
            observed = {
              assertionsGreen = pkgs.lib.all (a: a.assertion) cfg.config.assertions;
              observabilityEnabled =
                (builtins.fromJSON cfg.config.d2b._manifestPkg.text)._observability.enabled;
              stackVmDeclared =
                builtins.hasAttr "sys-obs" cfg.config.d2b.realms.local-root.workloads;
              workloadAgentDeclared =
                builtins.hasAttr "work-app" cfg.config.d2b.realms.work.workloads;
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
          # Keep fixture-dependent contract crates out of generic
          # sandbox workspace tests. Full D2B_FIXTURES delivery to the
          # sandbox/CI is a tracked W1 deliverable.
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
            ${self.packages.${system}.d2b-userd-static}/bin/d2b-userd \
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

        # Real cargo-deny gate: workspace-wide bans, licenses, and sources plus
        # focused broker and guest-shell policy configurations. Advisory
        # checks are handled by rust-audit below (cargo-deny requires
        # a fetchable URL for the advisory DB which is incompatible
        # with the Nix sandbox's no-network constraint).
        #
        # cargo-deny shells out to `cargo metadata`, so we vendor
        # the crate registry and override the sccache wrapper that
        # the repo-local .cargo/config.toml enables.
        rust-deny = pkgs.runCommand "d2b-rust-deny" {
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
            (
              cd "$ws"
              cargo-deny --manifest-path "$manifest" \
                check --config "$deny_cfg" bans licenses sources
            )
            rm -rf "$ws"
          }

          run_deny "main" \
            "${rustPackagesSrc}" \
            "Cargo.toml" \
            '${workspaceVendorConfig}' \
            "${rustPackagesSrc}/packages/deny.toml"

          run_deny "broker" \
            "${rustPackagesSrc}" \
            "d2b-priv-broker/Cargo.toml" \
            '${workspaceVendorConfig}' \
            "${rustPackagesSrc}/packages/d2b-priv-broker/deny.toml"

          run_deny "guest-shell-runner" \
            "${rustPackagesSrc}" \
            "d2b-guest-shell-runner/Cargo.toml" \
            '${workspaceVendorConfig}' \
            "${rustPackagesSrc}/packages/d2b-guest-shell-runner/deny.toml"

          echo ok > $out
        '';

        # Real cargo-audit gate: vulnerability scan of the canonical lockfile
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
            --ignore RUSTSEC-2026-0195 \
            --ignore RUSTSEC-2024-0384
          echo ok > $out
        '';

        guest-static-dependency-policy =
          pkgs.runCommand "d2b-guest-static-dependency-policy" {
            nativeBuildInputs = [ pkgs.cargo ];
          } ''
            cp -r ${rustPackagesSrc}/packages "$TMPDIR/workspace"
            chmod -R u+w "$TMPDIR/workspace"
            printf '%s\n' '${workspaceVendorConfig}' > "$TMPDIR/workspace/.cargo/config.toml"
            tree=$TMPDIR/guest.tree
            (
              cd "$TMPDIR/workspace"
              cargo tree --locked --offline --manifest-path Cargo.toml \
                --edges normal,build \
                -p d2b-guestd -p d2b-userd -p d2b-exec-runner -p d2b-sk-frontend
            ) > "$tree"
            if grep -E '(^|[[:space:]])(cc|cmake|pkg-config|openssl|openssl-sys|native-tls|libsystemd|systemd) v' "$tree"; then
              echo "guest static dependency closure contains a native-link/build-script dependency" >&2
              exit 1
            fi
            echo ok > "$out"
          '';

        guest-shell-runner-static-dependency-policy =
          pkgs.runCommand "d2b-guest-shell-runner-static-dependency-policy" {
            nativeBuildInputs = [ pkgs.cargo ];
          } ''
            cp -r ${rustPackagesSrc}/packages "$TMPDIR/workspace"
            chmod -R u+w "$TMPDIR/workspace"
            printf '%s\n' '${workspaceVendorConfig}' > "$TMPDIR/workspace/.cargo/config.toml"
            tree=$TMPDIR/guest-shell-runner.tree
            (
              cd "$TMPDIR/workspace"
              cargo tree --locked --offline --manifest-path Cargo.toml \
                -p d2b-guest-shell-runner --features real-libshpool \
                --edges normal,build
            ) > "$tree"
            if grep -E '(^|[[:space:]])(openssl|openssl-sys|native-tls|libsystemd|systemd|pam-sys|dlopen2) v' "$tree"; then
              echo "guest shell runner closure contains a forbidden dynamic/PAM/systemd dependency" >&2
              exit 1
            fi
            if ! grep -F 'motd v0.2.2' "$tree" >/dev/null; then
              echo "guest shell runner lock must pin the expected PAM-free motd dependency posture" >&2
              exit 1
            fi
            echo ok > "$out"
          '';

        delivery-tooling = pkgs.runCommand "d2b-delivery-tooling" {
          nativeBuildInputs = [
            pkgs.cmake
            pkgs.jq
            pkgs.pkg-config
            pkgs.sccache
            pkgs.stdenv.cc
            deliveryTools.stableRust
            deliveryTools.gh
            deliveryTools.gitTown
            deliveryTools.cargoUdepsNightly
            deliveryTools.cargoSemverChecks
          ];
          buildInputs = [ pkgs.openssl ];
        } ''
          gh --version | grep -F 'gh version 2.92.0'
          git-town --version | grep -Fx 'Git Town 23.0.1'
          cargo-udeps-nightly --version | grep -F 'cargo-udeps 0.1.61'
          cargo-semver-checks semver-checks --version \
            | grep -F 'cargo-semver-checks 0.47.0'
          rustc --version | grep -F 'rustc 1.94.1'
          clippy-driver -vV | grep -F 'release: 1.94.1'
          ${deliveryTools.nightlyRust}/bin/rustc --version \
            | grep -E '^rustc 1\.93\.0-nightly \([0-9a-f]+ 2025-11-30\)$'

          sccache --version
          export CARGO_NET_OFFLINE=true
          cargo metadata \
            --manifest-path ${./packages}/Cargo.toml \
            --locked \
            --offline \
            --no-deps \
            --format-version 1 > cargo-metadata.json
          jq -e '.packages | any(.name == "xtask")' cargo-metadata.json

          mkdir native-smoke
          cat > native-smoke/CMakeLists.txt <<'EOF'
          cmake_minimum_required(VERSION 3.20)
          project(d2b_delivery_native_smoke C)
          find_package(OpenSSL REQUIRED)
          add_executable(d2b-delivery-native-smoke main.c)
          target_link_libraries(d2b-delivery-native-smoke PRIVATE OpenSSL::Crypto)
          EOF
          cat > native-smoke/main.c <<'EOF'
          #include <openssl/crypto.h>
          int main(void) {
            return OpenSSL_version_num() == 0;
          }
          EOF
          cmake -S native-smoke -B native-smoke/build
          cmake --build native-smoke/build
          native-smoke/build/d2b-delivery-native-smoke

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
            # omits (TODO 1 — hardware-configuration). Without this
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

            # Sentinel overrides — these are the three fields gated
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
          (mkEval [
            (import ./examples/graphics-workstation/configuration.nix)
            {
              d2b.realms.desktop.broker = {
                enable = true;
                hostMutation = true;
              };
            }
          ]);
      });

      lib = nixpkgs.lib.makeExtensible (_: {
        inherit shippedRustPackages;
        supportedSystems = systems;
      });

      overlays.default = _final: _prev: { };
    };
}
