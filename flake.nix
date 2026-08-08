{
  description = "Opinionated NixOS desktop microVM workspaces";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

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

  outputs = { self, nixpkgs, fenix, home-manager, ... }@inputs:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      nixpkgsFor = forAllSystems (system: import nixpkgs { inherit system; });
      rustToolchainFile = ./packages/rust-toolchain.toml;
      rustToolchainManifestHash =
        "sha256-OATSZm98Es5kIFuqaba+UvkQtFsVgJEBMmS+t6od5/U=";
      rustToolchainChannel =
        (builtins.fromTOML (builtins.readFile rustToolchainFile)).toolchain.channel;
      rustToolchainComponents =
        (builtins.fromTOML (builtins.readFile rustToolchainFile)).toolchain.components;
      rustToolchainComponentNames = [ "cargo" "rustc" ] ++ rustToolchainComponents;
      mkRustToolchainComponents = system:
        fenix.packages.${system}.fromToolchainName {
          name = rustToolchainChannel;
          sha256 = rustToolchainManifestHash;
        };
      mkRustToolchain = system:
        let
          toolchain = mkRustToolchainComponents system;
        in
        toolchain.withComponents rustToolchainComponentNames;
      mkRustPlatform = system: pkgs:
        let
          toolchain = mkRustToolchainComponents system;
        in
        pkgs.makeRustPlatform {
          inherit (toolchain) cargo rustc;
        };
      mkStaticRustPlatform = system: pkgs:
        let
          toolchain = mkRustToolchainComponents system;
          target = pkgs.pkgsStatic.stdenv.targetPlatform.rust.rustcTarget;
          targetToolchain =
            fenix.packages.${system}.targets.${target}.fromToolchainName {
              name = rustToolchainChannel;
              sha256 = rustToolchainManifestHash;
            };
          staticToolchain = fenix.packages.${system}.combine [
            (toolchain.withComponents rustToolchainComponentNames)
            targetToolchain.rust-std
          ];
        in
        pkgs.pkgsStatic.makeRustPlatform {
          rustc = staticToolchain;
          cargo = staticToolchain;
        };
      mkBazelSeccomp = system:
        if builtins.elem system systems then
          import ./pkgs/bazel-8.6.0-seccomp {
            pkgs = nixpkgsFor.${system};
          }
        else
          throw ''
            D2B-BZLEXEC-NIX-PTRACE-SYSTEM: native-system is unsupported.
            Move evaluation and execution to a native x86_64-linux or aarch64-linux runner;
            run make test-flake; then rerun the exact closed slice command.
          '';
      mkRustsecAdvisoryDb = pkgs:
        let
          source = pkgs.fetchFromGitHub {
            owner = "rustsec";
            repo = "advisory-db";
            rev = "831c50f4a4304068f125e603add6a8839f08b3eb";
            hash = "sha256-wXKYURZz76ZC5lbuDA1oVQA/MxSB3pSJ1raF1HG0oIc=";
          };
        in
        pkgs.runCommand "rustsec-advisory-db-git" {
          nativeBuildInputs = [ pkgs.git ];
        } ''
          cp -r ${source} $out
          chmod -R u+w $out
          cd $out
          git init -q
          git add .
          git -c user.email=nixbld@localhost -c user.name=nixbld \
            commit -q -m 'advisory-db snapshot'
        '';
      mkGuestRustPackagesSrc = pkgs:
        pkgs.runCommand "d2b-guest-rust-src" { } ''
          mkdir -p $out/packages
          cp -r ${./packages/d2b-realm-core} $out/packages/d2b-realm-core
          cp -r ${./packages/d2b-core} $out/packages/d2b-core
          cp -r ${./packages/d2b-contracts} $out/packages/d2b-contracts
          cp -r ${./packages/d2b-guestd} $out/packages/d2b-guestd
          cp -r ${./packages/d2b-userd} $out/packages/d2b-userd
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
      # re-entry and toolchain bootstrap. Enter this shell and those paths are
      # skipped entirely, because the tools they look for are already present.
      #
      devShells = forAllSystems (system: let
        pkgs = nixpkgsFor.${system};
        bazelSeccomp = mkBazelSeccomp system;
        rustToolchain = mkRustToolchain system;
      in {
        default = pkgs.mkShell {
          name = "d2b-dev";
          packages = with pkgs; [
            # Toolchain. Fenix resolves the repository-pinned manifest and
            # supplies the exact compiler and cargo used by the gates.
            rustToolchain
            stdenv.cc
            # Bazel is the repository-pinned 8.6.0 output with the Linux
            # sandbox seccomp and PID-namespace monitor patch. Do not add an
            # ambient Bazel here.
            bazelSeccomp
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
            echo "d2b dev shell: rust $(sed -n 's/.*channel = "\(.*\)".*/\1/p' packages/rust-toolchain.toml) via Nix-pinned Fenix, sccache at $SCCACHE_DIR"
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
        rustPlatform = mkRustPlatform system pkgs;
        staticRustPlatform = mkStaticRustPlatform system pkgs;
        bazelSeccomp = mkBazelSeccomp system;
        bazelExecSupervisor =
          import ./pkgs/d2b-bazel-exec-supervisor { inherit pkgs; };
        rustsecAdvisoryDb = mkRustsecAdvisoryDb pkgs;
        rustPackagesSrc = pkgs.runCommand "d2b-rust-src" { } ''
          mkdir -p $out/packages
          cp -r ${./packages}/. $out/packages/
        '';
        rustWorkspace = args: rustPlatform.buildRustPackage ({
          pname = "d2b-rust-workspace";
          version = "0.0.0-bootstrap";
          src = rustPackagesSrc;
          sourceRoot = "d2b-rust-src/packages";
          cargoLock = {
            lockFile = ./packages/Cargo.lock;
            outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
          };
          # The execution crate accepts this only as a compiler environment
          # value. Keep the helper in the realized store and do not provide a
          # worktree, runfiles, or unhashed runtime fallback.
          D2B_BAZEL_EXEC_SUPERVISOR =
            "${bazelExecSupervisor}/bin/d2b-bazel-exec-supervisor";
          RUSTC_WRAPPER = "";
          SCCACHE_DIR = "";
        } // args // {
          nativeBuildInputs = [ pkgs.protobuf ] ++ (args.nativeBuildInputs or [ ]);
        });
        brokerHostPackage = rustPlatform.buildRustPackage {
          pname = "d2b-priv-broker";
          version = "0.0.0-bootstrap";
          src = rustPackagesSrc;
          sourceRoot = "d2b-rust-src/packages";
          cargoLock = {
            lockFile = ./packages/Cargo.lock;
            outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
          };
          D2B_BAZEL_EXEC_SUPERVISOR =
            "${bazelExecSupervisor}/bin/d2b-bazel-exec-supervisor";
          cargoBuildFlags = [
            "--package"
            "d2b-priv-broker"
            "--bin"
            "d2b-priv-broker"
            "--no-default-features"
          ];
          doCheck = false;
          RUSTC_WRAPPER = "";
          SCCACHE_DIR = "";
          postPatch = ''
            mkdir -p .cargo
            printf '%s\n' '[build]' 'rustc-wrapper = ""' > .cargo/config.toml
            rm -f .cargo/rustc-wrapper.sh
          '';
          meta.mainProgram = "d2b-priv-broker";
        };
        guestRustPackagesSrc = mkGuestRustPackagesSrc pkgs;
        cargoLock = {
          lockFile = ./packages/Cargo.guest.lock;
        };
        guestStaticPackage = packageName: binName:
          staticRustPlatform.buildRustPackage {
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
          staticRustPlatform.buildRustPackage {
            pname = "d2b-guest-shell-runner-static";
            version = "0.0.0-bootstrap";
            src = rustPackagesSrc;
            sourceRoot = "d2b-rust-src/packages";
            cargoLock = {
              lockFile = ./packages/Cargo.lock;
              outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
            };
            cargoBuildFlags = [
              "--package"
              "d2b-guest-shell-runner"
              "--bin"
              "d2b-guest-shell-runner"
              "--no-default-features"
              "--features"
              "real-libshpool"
            ];
            doCheck = false;
            RUSTC_WRAPPER = "";
            SCCACHE_DIR = "";
            RUSTFLAGS = "-C relocation-model=pie -C link-arg=-static-pie";
            nativeBuildInputs = [
              pkgs.pkgsStatic.binutils
              staticRustPlatform.bindgenHook
            ];
            postInstall = ''
              readelf=${pkgs.pkgsStatic.binutils.bintools}/bin/readelf
              bin="$out/bin/d2b-guest-shell-runner"
              test -x "$bin"
              "$readelf" -h "$bin" > "$TMPDIR/d2b-guest-shell-runner.header"
              grep -Eq '^[[:space:]]*Type:[[:space:]]+DYN([[:space:]]|$)' \
                "$TMPDIR/d2b-guest-shell-runner.header"
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
        "bazel-8.6.0-seccomp" = bazelSeccomp;
        "d2b-bazel-exec-supervisor" = bazelExecSupervisor;
        "rustsec-advisory-db" = rustsecAdvisoryDb;
        manpages = pkgs.runCommand "d2b-manpages" { } ''
          install -Dm644 ${./docs/manpages/d2b.1} "$out/share/man/man1/d2b.1"
          ${pkgs.gzip}/bin/gzip -n -c ${./docs/manpages/d2b.1} > "$out/share/man/man1/d2b.1.gz"
        '';

        completions = pkgs.runCommand "d2b-completions" { } ''
          install -Dm644 ${./docs/completions/d2b.bash} "$out/share/bash-completion/completions/d2b"
          install -Dm644 ${./docs/completions/d2b.zsh}  "$out/share/zsh/site-functions/_d2b"
          install -Dm644 ${./docs/completions/d2b.fish} "$out/share/fish/vendor_completions.d/d2b.fish"
        '';
        d2b-guestd-static = guestStaticPackage "d2b-guestd" "d2b-guestd";
        d2b-userd-static = guestStaticPackage "d2b-userd" "d2b-userd";
        d2b-exec-runner-static =
          guestStaticPackage "d2b-exec-runner" "d2b-exec-runner";
        d2b-sk-frontend-static =
          guestStaticPackage "d2b-sk-frontend" "d2b-sk-frontend";
        d2b-priv-broker = brokerHostPackage;
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
        rustToolchain = mkRustToolchainComponents system;
        rustPlatform = mkRustPlatform system pkgs;
        lib = pkgs.lib;
        bazelSeccomp = mkBazelSeccomp system;
        bazelExecSupervisor =
          import ./pkgs/d2b-bazel-exec-supervisor { inherit pkgs; };
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
        renderEvalFixture = evaluated: let
          bundle = evaluated.config.d2b._bundle;
          top = name: bundle.${name}.fixtureData;
        in {
          files = {
            "privileges.json" = top "privilegesJson";
            "host.json" = top "hostJson";
            "processes.json" = top "processesJson";
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
          closures = pkgs.lib.mapAttrs (_: closure: closure.data) bundle.closures;
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
          cp ${bundle.bundle.path} $out/bundle.json
          cp ${manifestPkg}/share/d2b/vms.json $out/manifest.json
          ${nixpkgs.lib.concatStringsSep "\n" (nixpkgs.lib.mapAttrsToList
            (vm: c: "cp ${c.path} $out/closures/${vm}.json")
            fullEval.config.d2b._bundle.closures)}
        '';
        evalFixtureData = {
          minimal = renderEvalFixture smokeEval;
          full = renderEvalFixture fullEval;
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
          mkdir -p $out/nixos-modules/components/observability
          cp -r ${./nixos-modules/components/observability/dashboards} \
            $out/nixos-modules/components/observability/dashboards
        '';
        guestRustPackagesSrc = mkGuestRustPackagesSrc pkgs;
        rustWorkspace = args: rustPlatform.buildRustPackage ({
          pname = "d2b-rust-workspace";
          version = "0.0.0-bootstrap";
          src = rustPackagesSrc;
          sourceRoot = "d2b-rust-src/packages";
          cargoLock = {
            lockFile = ./packages/Cargo.lock;
            outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
          };
          D2B_BAZEL_EXEC_SUPERVISOR =
            "${bazelExecSupervisor}/bin/d2b-bazel-exec-supervisor";
          # Repo-local .cargo/config.toml files set
          # `rustc-wrapper = "sccache"`, but the Nix sandbox doesn't
          # have sccache on PATH (and even if it did, sccache wants
          # a writable cache dir + network for distributed builds).
          # Disable the wrapper for sandbox builds. Operators running
          # cargo OUTSIDE the sandbox (worktrees, dev shells) still
          # get the sccache speedup from the config files.
          RUSTC_WRAPPER = "";
          SCCACHE_DIR = "";
        } // args // {
          nativeBuildInputs = [ pkgs.protobuf ] ++ (args.nativeBuildInputs or [ ]);
        });
        assertRustToolchain = ''
          rustc --version | grep -F "${rustToolchainChannel}"
        '';

        advisoryDbGit = mkRustsecAdvisoryDb pkgs;
        bazelToolchainGoldenPath =
          ./. + "/tests/golden/bazel-toolchain.json";
        bazelSupervisorGoldenPath =
          ./. + "/tests/golden/bazel-exec-supervisor.json";
        bazelToolchainGolden =
          if builtins.pathExists bazelToolchainGoldenPath
          then readJson bazelToolchainGoldenPath
          else null;
        bazelSupervisorGolden =
          if builtins.pathExists bazelSupervisorGoldenPath
          then readJson bazelSupervisorGoldenPath
          else null;
        currentBazelSourceHashes = {
          policy = builtins.hashFile "sha256"
            ./pkgs/bazel-8.6.0-seccomp/seccomp-policy.json;
          patch = builtins.hashFile "sha256"
            ./pkgs/bazel-8.6.0-seccomp/linux-sandbox-seccomp.patch;
          supervisor = builtins.hashFile "sha256"
            ./tests/tools/d2b-bazel-exec-supervisor/supervisor.c;
          plant = builtins.hashFile "sha256"
            ./tests/tools/d2b-bazel-exec-supervisor/sandbox-crash-plant.c;
          supervisorExpression = builtins.hashFile "sha256"
            ./pkgs/d2b-bazel-exec-supervisor/default.nix;
        };
        bazelSourceIdentityGate =
          if bazelToolchainGolden == null || bazelSupervisorGolden == null then
            throw "D2B-BZLTOOLCHAIN-IDENTITY"
          else if bazelToolchainGolden.patch.sha256
            != currentBazelSourceHashes.patch
          || bazelToolchainGolden.policy.sha256
            != currentBazelSourceHashes.policy
          || bazelSupervisorGolden.source.sha256
            != currentBazelSourceHashes.supervisor
          || bazelSupervisorGolden.source.plantSha256
            != currentBazelSourceHashes.plant
          || bazelSupervisorGolden.expression.sha256
            != currentBazelSourceHashes.supervisorExpression then
            throw "D2B-BZLTOOLCHAIN-IDENTITY"
          else if bazelSeccomp.passthru.d2bSeccomp.policySha256
            != currentBazelSourceHashes.policy then
            throw "D2B-BZLTOOLCHAIN-IDENTITY"
          else
            true;

        nativePrefix =
          if system == "x86_64-linux" then "x86_64"
          else if system == "aarch64-linux" then "aarch64"
          else throw "unsupported native system";
        nativeGnuTarget = "${nativePrefix}-unknown-linux-gnu";
        nativeMuslTarget = "${nativePrefix}-unknown-linux-musl";
        policyInputRoot = target: context:
          ./. + "/packages/policy-inputs/${system}/${target}/${context}";
        brokerPolicyRoot =
          policyInputRoot nativeGnuTarget "broker-production";
        guestPolicyRoot =
          policyInputRoot nativeMuslTarget "guest-real-libshpool";
        policyInputsPresent = builtins.pathExists
          (./. + "/packages/policy-inputs");
        getField = name: value:
          if builtins.isAttrs value && builtins.hasAttr name value
          then builtins.getAttr name value
          else null;
        uniqueList = values:
          builtins.isList values
          && builtins.length values == builtins.length (lib.unique values);
        exactList = expected: actual:
          builtins.isList actual
          && uniqueList actual
          && actual == expected;
        exactSet = expected: actual:
          builtins.isList actual
          && uniqueList actual
          && builtins.isList expected
          && uniqueList expected
          && lib.sort builtins.lessThan actual
            == lib.sort builtins.lessThan expected;
        nonemptyStrings = values:
          builtins.isList values
          && builtins.length values > 0
          && builtins.all builtins.isString values
          && builtins.all (value: value != "") values
          && uniqueList values;
        identityKey = value:
          let
            name = getField "name" value;
            version = getField "version" value;
            source = getField "source" value;
          in
          if builtins.isAttrs value
            && builtins.isString name
            && name != ""
            && builtins.isString version
            && version != ""
            && (source == null || builtins.isString source)
          then "${name}|${version}|${if source == null then "" else source}"
          else null;
        identityKeys = values:
          if builtins.isList values then map identityKey values else [ ];
        validIdentityKeys = values:
          builtins.isList values
          && builtins.length values > 0
          && builtins.all (value: value != null) values
          && uniqueList values;
        identitiesEqual = left: right:
          let
            leftKeys = identityKeys left;
            rightKeys = identityKeys right;
          in
          validIdentityKeys leftKeys
          && validIdentityKeys rightKeys
          && exactSet leftKeys rightKeys;
        readJson = path: builtins.fromJSON (builtins.readFile path);
        readToml = path: builtins.fromTOML (builtins.readFile path);
        lockPackageIdentities = lock:
          let packages = getField "package" lock;
          in if builtins.isList packages
          then map (package: {
            name = getField "name" package;
            version = getField "version" package;
            source = getField "source" package;
          }) packages
          else [ ];
        lockDependenciesClosed = lock:
          let
            packages = getField "package" lock;
            names =
              if builtins.isList packages
              then map (package: getField "name" package) packages
              else [ ];
            dependencyName = token:
              let parts = lib.splitString " " token;
              in if parts == [ ] then "" else builtins.head parts;
            packageOk = package:
              let dependencies = getField "dependencies" package;
              in dependencies == null
                || (builtins.isList dependencies
                  && builtins.all (token:
                    builtins.isString token
                    && token != ""
                    && builtins.elem (dependencyName token) names)
                    dependencies);
          in
          builtins.isAttrs lock
          && builtins.isList packages
          && builtins.length packages > 0
          && builtins.all packageOk packages;
        lockMatches = lock: identities:
          let
            lockIdentities = lockPackageIdentities lock;
            lockKeys = identityKeys lockIdentities;
            identityKeysExpected = identityKeys identities;
          in
          lockDependenciesClosed lock
          && validIdentityKeys lockKeys
          && exactSet identityKeysExpected lockKeys;
        packageForId = packages: id:
          lib.findFirst
            (package: getField "id" package == id)
            null
            packages;
        nodeForId = nodes: id:
          lib.findFirst
            (node: getField "id" node == id)
            null
            nodes;
        edgeKind = kind:
          let value = getField "kind" kind;
          in if value == null then "normal" else value;
        edgeKey = edge:
          let
            package = getField "pkg" edge;
            name = getField "name" edge;
            kinds = getField "dep_kinds" edge;
          in
          if builtins.isString package
            && builtins.isString name
            && builtins.isList kinds
          then "${package}|${name}|${builtins.toJSON kinds}"
          else null;
        resolveNodeEdgesClosed = { node, nodeIds, packages, allowedKinds }:
          let
            dependencies = getField "dependencies" node;
            dependenciesOk =
              builtins.isList dependencies
              && builtins.all builtins.isString dependencies
              && uniqueList dependencies
              && builtins.all (id: builtins.elem id nodeIds) dependencies;
            details = getField "deps" node;
            detailIds =
              if builtins.isList details
              then map (detail: getField "pkg" detail) details
              else [ ];
            detailKeys =
              if builtins.isList details
              then map edgeKey details
              else [ ];
            detailsOk =
              builtins.isList details
              && builtins.all (detail:
                let
                  packageId = getField "pkg" detail;
                  packageName = getField "name" detail;
                  kinds = getField "dep_kinds" detail;
                  target = getField "target" detail;
                  targetPackage = packageForId packages packageId;
                in
                builtins.isAttrs detail
                && builtins.isString packageId
                && builtins.elem packageId nodeIds
                && builtins.isString packageName
                && builtins.isAttrs targetPackage
                && packageName != ""
                && (target == null || builtins.isString target)
                && builtins.isList kinds
                && builtins.length kinds > 0
                && builtins.all (kind:
                  builtins.isAttrs kind
                  && (getField "kind" kind == null
                    || builtins.isString (getField "kind" kind))
                  && ((getField "target" kind) == null
                    || builtins.isString (getField "target" kind))
                  && builtins.elem (edgeKind kind) allowedKinds)
                  kinds)
                details
              && builtins.all (key: key != null) detailKeys
              && uniqueList detailKeys
              && uniqueList detailIds
              && exactSet dependencies detailIds;
          in
          dependenciesOk && detailsOk;
        reachableNodeIds = nodes: seen: frontier:
          if frontier == [ ] then
            seen
          else
            let
              fresh = lib.filter (id: !(builtins.elem id seen)) frontier;
              next = lib.concatMap (id:
                let node = nodeForId nodes id;
                in if node == null
                then [ ]
                else getField "dependencies" node)
                fresh;
            in
            reachableNodeIds nodes (lib.unique (seen ++ fresh)) next;
        policyArtifactShapeOk =
          { artifact
          , lock
          , expected
          , variant
          , expectedEdgeKinds
          }:
          let
            packages = getField "packages" artifact;
            identities = getField "identities" artifact;
            resolve = getField "resolve" artifact;
            nodes = if builtins.isAttrs resolve
              then getField "nodes" resolve
              else null;
            packageIds =
              if builtins.isList packages
              then map (package: getField "id" package) packages
              else [ ];
            nodeIds =
              if builtins.isList nodes
              then map (node: getField "id" node) nodes
              else [ ];
            rootPackages =
              if builtins.isList packages
              then lib.filter
                (package: getField "name" package == expected.package)
                packages
              else [ ];
            resolveRoot = if builtins.isAttrs resolve
              then getField "root" resolve
              else null;
            rootNodes =
              if builtins.isList nodes
              then lib.filter (node: getField "id" node == resolveRoot) nodes
              else [ ];
            rootPackageId =
              if builtins.length rootPackages == 1
              then getField "id" (builtins.head rootPackages)
              else null;
            graphOk =
              builtins.isAttrs resolve
              && builtins.isList packages
              && builtins.length packages > 0
              && builtins.all (package:
                builtins.isAttrs package
                && identityKey package != null
                && builtins.isString (getField "id" package)
                && getField "id" package != "")
                packages
              && uniqueList packageIds
              && builtins.isList nodes
              && builtins.length nodes > 0
              && builtins.all (node:
                builtins.isAttrs node
                && builtins.isString (getField "id" node)
                && getField "id" node != "")
                nodes
              && uniqueList nodeIds
              && exactSet packageIds nodeIds
              && builtins.all (node:
                resolveNodeEdgesClosed {
                  inherit node nodeIds packages;
                  allowedKinds = lib.splitString "," expectedEdgeKinds;
                })
                nodes
              && builtins.isString resolveRoot
              && builtins.elem resolveRoot nodeIds
              && builtins.length rootPackages == 1
              && builtins.length rootNodes == 1
              && rootPackageId == resolveRoot
              && exactSet nodeIds (reachableNodeIds nodes [ ] [ resolveRoot ]);
          in
          builtins.isAttrs artifact
          && getField "system" artifact == expected.system
          && getField "target" artifact == expected.target
          && getField "package" artifact == expected.package
          && getField "root" artifact == expected.package
          && getField "variant" artifact == variant
          && getField "edgeKinds" artifact == expectedEdgeKinds
          && getField "defaultFeatures" artifact == false
          && exactList expected.features (getField "features" artifact)
          && builtins.isString (getField "sourceCensusSha256" artifact)
          && hexDigest (getField "sourceCensusSha256" artifact)
          && identitiesEqual identities packages
          && graphOk
          && lockMatches lock packages;
        productionArtifactShapeOk =
          { artifact
          , lock
          , expected
          , policyArtifact
          }:
          let
            identities = getField "identities" artifact;
            identityKeysValue = identityKeys identities;
            policyIdentityKeys =
              identityKeys (getField "packages" policyArtifact);
          in
          builtins.isAttrs artifact
          && getField "system" artifact == expected.system
          && getField "target" artifact == expected.target
          && getField "package" artifact == expected.package
          && getField "root" artifact == expected.package
          && getField "variant" artifact == "production"
          && getField "edgeKinds" artifact == "normal,build"
          && getField "defaultFeatures" artifact == false
          && exactList expected.features (getField "features" artifact)
          && validIdentityKeys identityKeysValue
          && exactSet identityKeysValue identityKeysValue
          && builtins.all (key: builtins.elem key policyIdentityKeys)
              identityKeysValue
          && lockMatches lock identities;
        policyContexts = [
          {
            system = "x86_64-linux";
            target = "x86_64-unknown-linux-gnu";
            context = "broker-production";
            package = "d2b-priv-broker";
            features = [ ];
          }
          {
            system = "x86_64-linux";
            target = "x86_64-unknown-linux-musl";
            context = "guest-real-libshpool";
            package = "d2b-guest-shell-runner";
            features = [ "real-libshpool" ];
          }
          {
            system = "aarch64-linux";
            target = "aarch64-unknown-linux-gnu";
            context = "broker-production";
            package = "d2b-priv-broker";
            features = [ ];
          }
          {
            system = "aarch64-linux";
            target = "aarch64-unknown-linux-musl";
            context = "guest-real-libshpool";
            package = "d2b-guest-shell-runner";
            features = [ "real-libshpool" ];
          }
        ];
        policyContextRoot = context:
          ./. + "/packages/policy-inputs/${context.system}/${context.target}/${context.context}";
        readPolicyContext = context:
          let root = policyContextRoot context;
          in {
            inherit context root;
            production = readJson (root + "/production/closure.json");
            productionLock = readToml (root + "/production/Cargo.lock");
            policy = readJson (root + "/policy/metadata.json");
            policyLock = readToml (root + "/policy/Cargo.lock");
          };
        policyContextRecords =
          if policyInputsPresent
          then map readPolicyContext policyContexts
          else [ ];
        policyContextKeys = records:
          map (record:
            let context = record.context;
            in "${context.system}/${context.target}/${context.context}/${context.package}/${context.package}")
            records;
        policyContextShapeOk = record:
          let
            context = record.context;
            expected = {
              system = context.system;
              target = context.target;
              package = context.package;
              features = context.features;
            };
          in
          policyArtifactShapeOk {
            artifact = record.policy;
            lock = record.policyLock;
            inherit expected;
            variant = "policy";
            expectedEdgeKinds = "normal,build,dev";
          }
          && productionArtifactShapeOk {
            artifact = record.production;
            lock = record.productionLock;
            policyArtifact = record.policy;
            inherit expected;
          };
        policyInputCorpusShapeOk =
          policyInputsPresent
          && builtins.length policyContextRecords
            == builtins.length policyContexts
          && uniqueList (policyContextKeys policyContextRecords)
          && builtins.length (lib.unique (policyContextKeys policyContextRecords))
            == builtins.length policyContexts
          && builtins.all policyContextShapeOk policyContextRecords;
        policyInputCorpusGate =
          if policyInputCorpusShapeOk then true
          else throw "D2B-BZLPOLICY-INPUT";
        artifactBaselinePath =
          ./. + "/tests/golden/bazel-rust-artifact-baselines.json";
        artifactBaselines =
          if builtins.pathExists artifactBaselinePath
          then builtins.fromJSON (builtins.readFile artifactBaselinePath)
          else null;
        artifactRows =
          if artifactBaselines == null
          then [ ]
          else artifactBaselines.rows or [ ];
        expectedArtifactPairs = [
          "x86_64-linux/broker-host-artifact-contract"
          "x86_64-linux/guest-static-elf"
          "aarch64-linux/broker-host-artifact-contract"
          "aarch64-linux/guest-static-elf"
        ];
        artifactPair = row: "${row.system}/${row.artifact}";
        artifactAuthFields = [
          "artifact"
          "candidateContentSha256"
          "decision"
          "deltaBytes"
          "newBinaryBytes"
          "priorBinaryBytes"
          "rationalePath"
          "reviewRecordSha256"
          "system"
        ];
        hexDigest = value:
          builtins.isString value
          && builtins.match "[0-9a-fA-F]{64}" value != null;
        rationalePathOk = authorization:
          let
            path = getField "rationalePath" authorization;
            components =
              if builtins.isString path then lib.splitString "/" path else [ ];
            repositoryPath =
              if builtins.isString path then ./. + "/${path}" else null;
          in
          builtins.isString path
          && path != ""
          && !(lib.hasPrefix "/" path)
          && builtins.all
            (component: component != "" && component != "." && component != "..")
            components
          && builtins.pathExists repositoryPath;
        reviewRecordMatches = authorization:
          rationalePathOk authorization
          && hexDigest (getField "reviewRecordSha256" authorization)
          && builtins.hashFile "sha256"
            (./. + "/${authorization.rationalePath}")
            == authorization.reviewRecordSha256;
        artifactAuthorizationShapeOk = row:
          let
            authorization = getField "sizeGrowthAuthorization" row;
          in
          authorization == null
          || (builtins.isAttrs authorization
            && lib.sort builtins.lessThan (builtins.attrNames authorization)
              == lib.sort builtins.lessThan artifactAuthFields
            && authorization.system == getField "system" row
            && authorization.artifact == getField "artifact" row
            && authorization.decision == "approved"
            && builtins.isInt authorization.priorBinaryBytes
            && authorization.priorBinaryBytes == getField "binaryBytes" row
            && builtins.isInt authorization.newBinaryBytes
            && authorization.newBinaryBytes > authorization.priorBinaryBytes
            && builtins.isInt authorization.deltaBytes
            && authorization.deltaBytes
              == authorization.newBinaryBytes - authorization.priorBinaryBytes
            && hexDigest authorization.candidateContentSha256
            && rationalePathOk authorization
            && reviewRecordMatches authorization);
        artifactLinkageShapeOk = row:
          let
            system = getField "system" row;
            artifact = getField "artifact" row;
            expectedMachine =
              if system == "x86_64-linux" then "EM_X86_64"
              else if system == "aarch64-linux" then "EM_AARCH64"
              else "";
            expectedInterpreter =
              if system == "x86_64-linux" then "ld-linux-x86-64.so.2"
              else if system == "aarch64-linux" then "ld-linux-aarch64.so.1"
              else "";
            expectedNeeded =
              if system == "x86_64-linux" then [
                "ld-linux-x86-64.so.2"
                "libc.so.6"
                "libgcc_s.so.1"
                "libm.so.6"
              ] else if system == "aarch64-linux" then [
                "ld-linux-aarch64.so.1"
                "libc.so.6"
                "libgcc_s.so.1"
                "libm.so.6"
              ] else [ ];
          in
          getField "elfType" row == "ET_DYN"
          && getField "elfMachine" row == expectedMachine
          && (if artifact == "broker-host-artifact-contract" then
            getField "interpreter" row == expectedInterpreter
            && exactList expectedNeeded (getField "needed" row)
          else
            getField "interpreter" row == null
            && exactList [ ] (getField "needed" row));
        authorizationRows =
          lib.filter
            (row: getField "sizeGrowthAuthorization" row != null)
            artifactRows;
        authorizationPaths =
          map (row:
            (getField "sizeGrowthAuthorization" row).rationalePath)
            authorizationRows;
        authorizationReviewDigests =
          map (row:
            (getField "sizeGrowthAuthorization" row).reviewRecordSha256)
            authorizationRows;
        authorizationCandidates =
          map (row:
            (getField "sizeGrowthAuthorization" row).candidateContentSha256)
            authorizationRows;
        artifactBaselineShapeOk =
          artifactBaselines != null
          && builtins.isAttrs artifactBaselines
          && builtins.isList artifactRows
          && builtins.length artifactRows == 4
          && uniqueList (map artifactPair artifactRows)
          && lib.sort builtins.lessThan (map artifactPair artifactRows)
            == lib.sort builtins.lessThan expectedArtifactPairs
          && !(lib.hasInfix storePathMarker (builtins.toJSON artifactBaselines))
          && builtins.all (row:
            builtins.isAttrs row
            && builtins.isString (getField "system" row)
            && builtins.isString (getField "artifact" row)
            && builtins.isInt (getField "binaryBytes" row)
            && getField "binaryBytes" row > 0
            && hexDigest (getField "binarySha256" row)
            && builtins.isInt (getField "closureCount" row)
            && getField "closureCount" row > 0
            && hexDigest (getField "closureSha256" row)
            && hexDigest (getField "selectedPolicyDigest" row)
            && builtins.isString (getField "measurementCommand" row)
            && getField "measurementCommand" row != ""
            && builtins.isString (getField "candidateCommit" row)
            && builtins.match "[0-9a-fA-F]{40}" (getField "candidateCommit" row) != null
            && artifactLinkageShapeOk row
            && !(builtins.hasAttr "rowAllowance" row)
            && !(builtins.hasAttr "sizeAllowance" row)
            && builtins.hasAttr "sizeGrowthAuthorization" row
            && artifactAuthorizationShapeOk row)
            artifactRows
          && uniqueList authorizationPaths
          && uniqueList authorizationReviewDigests
          && uniqueList authorizationCandidates;
        artifactBaselineGate =
          if artifactBaselineShapeOk then true
          else throw "D2B-BZLARTIFACT-IDENTITY";
        storePathMarker = lib.concatStringsSep "/" [ "" "nix" "store" "" ];
        baselineRowFor = artifact:
          lib.findFirst
            (row: row.system == system && row.artifact == artifact)
            null
            artifactRows;
        artifactContractPrelude = ''
          set -euo pipefail
          fail() {
            printf '%s\n' "$1" >&2
            exit 1
          }
        '';
        mkArtifactContract =
          { artifact
          , binaryPackage
          , binaryName
          , policyRoot
          , row
          , guest
          }:
          if !bazelSourceIdentityGate
            || !policyInputsPresent
            || !artifactBaselineGate
            || row == null then
            pkgs.runCommand "d2b-${artifact}-baseline-input" { } ''
              printf '%s\n' D2B-BZLARTIFACT-IDENTITY >&2
              exit 1
            ''
          else
            let
              binary = "${binaryPackage}/bin/${binaryName}";
              closureInfo = pkgs.closureInfo {
                rootPaths = [ binaryPackage ];
              };
              expectedMachine =
                if system == "x86_64-linux"
                then "Advanced Micro Devices X86-64"
                else "AArch64";
              expectedE =
                if system == "x86_64-linux" then "EM_X86_64" else "EM_AARCH64";
              expectedInterpreter = getField "interpreter" row;
              expectedNeeded =
                lib.concatStringsSep "\n" (getField "needed" row);
              authorization = getField "sizeGrowthAuthorization" row;
              authorizationDecision =
                if authorization == null then "" else getField "decision" authorization;
              authorizationSystem =
                if authorization == null then "" else getField "system" authorization;
              authorizationArtifact =
                if authorization == null then "" else getField "artifact" authorization;
              authorizationPrior =
                if authorization == null then 0
                else getField "priorBinaryBytes" authorization;
              authorizationNew =
                if authorization == null then 0
                else getField "newBinaryBytes" authorization;
              authorizationDelta =
                if authorization == null then 0
                else getField "deltaBytes" authorization;
              authorizationRationale =
                if authorization == null then ""
                else getField "rationalePath" authorization;
              authorizationCandidate =
                if authorization == null then ""
                else getField "candidateContentSha256" authorization;
              authorizationReview =
                if authorization == null then ""
                else getField "reviewRecordSha256" authorization;
              authorizationReviewDigest =
                if authorization == null then ""
                else builtins.hashFile "sha256"
                  (./. + "/${authorization.rationalePath}");
            in
            pkgs.runCommand "d2b-${artifact}-contract" {
              nativeBuildInputs = [
                pkgs.binutils
                pkgs.coreutils
                pkgs.gnugrep
                pkgs.gawk
              ];
            } ''
              ${artifactContractPrelude}
              binary=${lib.escapeShellArg binary}
              test -x "$binary" || fail D2B-BZLARTIFACT-IDENTITY
              header="$TMPDIR/header"
              program_headers="$TMPDIR/program-headers"
              dynamic="$TMPDIR/dynamic"
              dynamic_error="$TMPDIR/dynamic-error"
              ${pkgs.binutils.bintools}/bin/readelf -h "$binary" > "$header" \
                || fail D2B-BZLARTIFACT-LINKAGE
              grep -Eq "^[[:space:]]*Machine:[[:space:]]+${expectedMachine}$" "$header" \
                || fail D2B-BZLARTIFACT-LINKAGE
              actual_elf_type=$(
                ${pkgs.gawk}/bin/awk '/^[[:space:]]*Type:/ { print $2; exit }' "$header"
              )
              case "$actual_elf_type" in
                DYN) actual_elf_type=ET_DYN ;;
                EXEC) actual_elf_type=ET_EXEC ;;
                REL) actual_elf_type=ET_REL ;;
                *) actual_elf_type=unknown ;;
              esac
              test "$actual_elf_type" = ${lib.escapeShellArg row.elfType} \
                || fail D2B-BZLARTIFACT-LINKAGE
              actual_e_machine=$(
                case ${lib.escapeShellArg expectedMachine} in
                  "Advanced Micro Devices X86-64") printf '%s\n' EM_X86_64 ;;
                  "AArch64") printf '%s\n' EM_AARCH64 ;;
                  *) printf '%s\n' unknown ;;
                esac
              )
              printf '%s\n' ${lib.escapeShellArg expectedE} > "$TMPDIR/expected-e-machine"
              test "$actual_e_machine" = "$(cat "$TMPDIR/expected-e-machine")" \
                || fail D2B-BZLARTIFACT-LINKAGE
              test "$actual_e_machine" = ${lib.escapeShellArg row.elfMachine} \
                || fail D2B-BZLARTIFACT-LINKAGE
              # Guest artifacts are ET_DYN static PIE and have no PT_INTERP or DT_NEEDED.
              ${if guest then ''
                grep -Eq '^[[:space:]]*Type:[[:space:]]+DYN([[:space:]]|$)' "$header" \
                  || fail D2B-BZLARTIFACT-LINKAGE
                ${pkgs.binutils.bintools}/bin/readelf -l "$binary" > "$program_headers" \
                  || fail D2B-BZLARTIFACT-LINKAGE
                if grep -Fq 'Requesting program interpreter' "$program_headers"; then
                  fail D2B-BZLARTIFACT-LINKAGE
                fi
                if ${pkgs.binutils.bintools}/bin/readelf -d "$binary" > "$dynamic" \
                  2> "$dynamic_error"; then
                  if grep -Fq '(NEEDED)' "$dynamic"; then
                    fail D2B-BZLARTIFACT-LINKAGE
                  fi
                elif ! grep -qi 'no dynamic section' "$dynamic_error"; then
                  fail D2B-BZLARTIFACT-LINKAGE
                fi
              '' else ''
                ${pkgs.binutils.bintools}/bin/readelf -l "$binary" > "$program_headers" \
                  || fail D2B-BZLARTIFACT-LINKAGE
                actual_interpreter=$(
                  sed -n 's/.*Requesting program interpreter: \([^]]*\)].*/\1/p' \
                    "$program_headers" | sed 's#.*/##'
                )
                test "$actual_interpreter" = ${lib.escapeShellArg expectedInterpreter} \
                  || fail D2B-BZLARTIFACT-LINKAGE
                ${pkgs.binutils.bintools}/bin/readelf -d "$binary" > "$dynamic" \
                  2> "$dynamic_error" || fail D2B-BZLARTIFACT-LINKAGE
                actual_needed=$(
                  sed -n 's/.*Shared library: \[\([^]]*\)\].*/\1/p' "$dynamic" | sort
                )
                test "$actual_needed" = ${lib.escapeShellArg expectedNeeded} \
                  || fail D2B-BZLARTIFACT-LINKAGE
              ''}

              actual_bytes=$(${pkgs.coreutils}/bin/stat -c %s "$binary")
              actual_binary_sha=$(${pkgs.coreutils}/bin/sha256sum "$binary" \
                | ${pkgs.gawk}/bin/awk '{print $1}')
              test "$actual_binary_sha" = ${lib.escapeShellArg row.binarySha256} \
                || fail D2B-BZLARTIFACT-IDENTITY
              baseline_bytes=${toString row.binaryBytes}
              if test "$actual_bytes" -gt "$baseline_bytes"; then
                test ${lib.escapeShellArg authorizationDecision} = approved \
                  || fail D2B-BZLARTIFACT-SIZE-AUTH
                test ${lib.escapeShellArg authorizationSystem} = ${lib.escapeShellArg system} \
                  || fail D2B-BZLARTIFACT-SIZE-AUTH
                test ${lib.escapeShellArg authorizationArtifact} = ${lib.escapeShellArg artifact} \
                  || fail D2B-BZLARTIFACT-SIZE-AUTH
                test ${toString authorizationPrior} -eq "$baseline_bytes" \
                  || fail D2B-BZLARTIFACT-SIZE-AUTH
                test ${toString authorizationNew} -eq "$actual_bytes" \
                  || fail D2B-BZLARTIFACT-SIZE-AUTH
                test ${toString authorizationDelta} -eq \
                  "$((actual_bytes - baseline_bytes))" \
                  || fail D2B-BZLARTIFACT-SIZE-AUTH
                test ${toString authorizationDelta} -gt 0 \
                  || fail D2B-BZLARTIFACT-SIZE-AUTH
                case ${lib.escapeShellArg authorizationRationale} in
                  ""|/*|*//*|.|./*|*/.|../*|*/../*|*/..) \
                    fail D2B-BZLARTIFACT-SIZE-AUTH ;;
                esac
                printf '%s\n' ${lib.escapeShellArg authorizationCandidate} \
                  | grep -Eq '^[0-9a-fA-F]{64}$' \
                  || fail D2B-BZLARTIFACT-SIZE-AUTH
                test ${lib.escapeShellArg authorizationCandidate} = "$actual_binary_sha" \
                  || fail D2B-BZLARTIFACT-SIZE-AUTH
                printf '%s\n' ${lib.escapeShellArg authorizationReview} \
                  | grep -Eq '^[0-9a-fA-F]{64}$' \
                  || fail D2B-BZLARTIFACT-SIZE-AUTH
                test ${lib.escapeShellArg authorizationReview} = \
                  ${lib.escapeShellArg authorizationReviewDigest} \
                  || fail D2B-BZLARTIFACT-SIZE-AUTH
              else
                test ${lib.escapeShellArg authorizationDecision} = "" \
                  || fail D2B-BZLARTIFACT-SIZE-AUTH
              fi

              closure_count=$(${pkgs.coreutils}/bin/wc -l < ${closureInfo}/store-paths)
              closure_count=$(${pkgs.coreutils}/bin/tr -d '[:space:]' <<< "$closure_count")
              closure_sha=$(${pkgs.coreutils}/bin/sha256sum ${closureInfo}/store-paths | ${pkgs.gawk}/bin/awk '{print $1}')
              test "$closure_count" -eq ${toString row.closureCount} \
                || fail D2B-BZLARTIFACT-CLOSURE
              test "$closure_sha" = ${lib.escapeShellArg row.closureSha256} \
                || fail D2B-BZLARTIFACT-CLOSURE
              policy_sha=$(${pkgs.coreutils}/bin/sha256sum \
                ${lib.escapeShellArg "${policyRoot}/policy/metadata.json"} \
                | ${pkgs.gawk}/bin/awk '{print $1}')
              test "$policy_sha" = ${lib.escapeShellArg row.selectedPolicyDigest} \
                || fail D2B-BZLARTIFACT-IDENTITY
              mkdir -p "$out"
              printf '%s\n' ok > "$out/result"
            '';
        mkPolicyInputCheck =
          { name
          , root
          , package
          , target
          , feature
          , variant
          , edgeKinds
          , production
          }:
          let
            expectedContext = lib.findFirst
              (context:
                context.system == system
                && context.target == target
                && context.package == package
                && context.features
                  == (if feature == "" then [ ] else [ feature ]))
              null
              policyContexts;
            contextRecord =
              if expectedContext == null then null
              else lib.findFirst
                (record:
                  record.context.system == system
                  && record.context.target == target
                  && record.context.package == package
                  && record.context.context == expectedContext.context)
                null
                policyContextRecords;
            contextBound =
              contextRecord != null
              && toString contextRecord.root == toString root
              && variant == (if production then "production" else "policy")
              && edgeKinds
                == (if production then "normal,build" else "normal,build,dev");
          in
          if !policyInputsPresent
            || !policyInputCorpusGate
            || expectedContext == null || !contextBound then
            pkgs.runCommand "d2b-${name}-input" { } ''
              printf '%s\n' D2B-BZLPOLICY-INPUT >&2
              exit 1
            ''
          else
            pkgs.runCommand "d2b-${name}" {
            } ''
              set -euo pipefail
              mkdir -p "$out"
              printf '%s\n' ok > "$out/result"
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
            "bazel-package-policy.nix"
            "bazel-toolchain.nix"
            "examples-with-observability.nix"
            "ifname-nix-rust-parity.nix"
            "observability.nix"
            "provider-catalog.nix"
            "readiness-waves.nix"
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
          cargoBuildFlags = [
            "--workspace"
            "--exclude"
            "d2b-priv-broker"
            "--exclude"
            "d2b-guest-shell-runner"
          ];
          doCheck = false;
        };

        rust-tests = rustWorkspace {
          pname = "d2b-rust-tests";
          preBuild = assertRustToolchain;
          cargoBuildFlags = [
            "--workspace"
            "--exclude"
            "d2b-priv-broker"
            "--exclude"
            "d2b-guest-shell-runner"
          ];
          # Keep fixture-dependent contract crates out of generic sandbox
          # workspace tests. fixture-smoke only renders their input artifacts;
          # it does not execute these tests. Full D2B_FIXTURES delivery to the
          # sandbox/CI is tracked separately.
          cargoTestFlags = [
            "--workspace"
            "--exclude"
            "d2b-contract-tests"
            "--exclude"
            "d2b-priv-broker"
            "--exclude"
            "d2b-guest-shell-runner"
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
          nativeBuildInputs = [ rustToolchain.clippy ];
          cargoBuildFlags = [
            "--workspace"
            "--exclude"
            "d2b-priv-broker"
            "--exclude"
            "d2b-guest-shell-runner"
          ];
          doCheck = false;
          buildPhase = ''
            runHook preBuild
            ${assertRustToolchain}
            cargo clippy --workspace --all-targets \
              --exclude d2b-priv-broker \
              --exclude d2b-guest-shell-runner \
              -- -D warnings
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            mkdir -p $out
            echo ok > $out/rust-clippy
            runHook postInstall
          '';
        };

        broker-host-artifact-contract =
          mkArtifactContract {
            artifact = "broker-host-artifact-contract";
            binaryPackage = self.packages.${system}."d2b-priv-broker";
            binaryName = "d2b-priv-broker";
            policyRoot = brokerPolicyRoot;
            row = baselineRowFor "broker-host-artifact-contract";
            guest = false;
          };

        guest-static-elf =
          if !policyInputsPresent || !artifactBaselineShapeOk
            || baselineRowFor "guest-static-elf" == null then
            pkgs.runCommand "d2b-guest-static-elf-baseline-input" { } ''
              printf '%s\n' D2B-BZLARTIFACT-IDENTITY >&2
              exit 1
            ''
          else
            let
              guestContract = mkArtifactContract {
                artifact = "guest-static-elf";
                binaryPackage =
                  self.packages.${system}."d2b-guest-shell-runner-static";
                binaryName = "d2b-guest-shell-runner";
                policyRoot = guestPolicyRoot;
                row = baselineRowFor "guest-static-elf";
                guest = true;
              };
            in
            pkgs.runCommand "d2b-guest-static-elf" {
              nativeBuildInputs = [ pkgs.pkgsStatic.binutils ];
            } ''
              set -euo pipefail
              fail() {
                printf '%s\n' "$1" >&2
                exit 1
              }
              readelf=${pkgs.pkgsStatic.binutils.bintools}/bin/readelf
              for bin in \
                ${self.packages.${system}.d2b-guestd-static}/bin/d2b-guestd \
                ${self.packages.${system}.d2b-userd-static}/bin/d2b-userd \
                ${self.packages.${system}.d2b-exec-runner-static}/bin/d2b-exec-runner \
                ${self.packages.${system}.d2b-sk-frontend-static}/bin/d2b-sk-frontend \
                ${self.packages.${system}."d2b-guest-shell-runner-static"}/bin/d2b-guest-shell-runner
              do
                test -x "$bin" || fail D2B-BZLARTIFACT-IDENTITY
                name="$(basename "$bin")"
                "$readelf" -h "$bin" > "$TMPDIR/$name.header" \
                  || fail D2B-BZLARTIFACT-LINKAGE
                "$readelf" -l "$bin" > "$TMPDIR/$name.program-headers" \
                  || fail D2B-BZLARTIFACT-LINKAGE
                if grep -q 'Requesting program interpreter' "$TMPDIR/$name.program-headers"; then
                  fail D2B-BZLARTIFACT-LINKAGE
                fi
                if "$readelf" -d "$bin" > "$TMPDIR/$name.dynamic" \
                  2> "$TMPDIR/$name.dynamic.err"; then
                  if grep -q '(NEEDED)' "$TMPDIR/$name.dynamic"; then
                    fail D2B-BZLARTIFACT-LINKAGE
                  fi
                elif ! grep -qi 'no dynamic section' "$TMPDIR/$name.dynamic.err"; then
                  fail D2B-BZLARTIFACT-LINKAGE
                fi
              done
              test -f ${guestContract}/result || fail D2B-BZLARTIFACT-IDENTITY
              mkdir -p "$out"
              printf '%s\n' ok > "$out/guest-static-elf"
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
          mainVendor = rustPlatform.importCargoLock {
            lockFile = ./packages/Cargo.lock;
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
          nativeBuildInputs = [
            pkgs.cargo-deny
            rustToolchain.cargo
            rustToolchain.rustc
          ];
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

          echo ok > $out
        '';

        guest-rust-deny = let
          guestVendor = rustPlatform.importCargoLock {
            lockFile = ./packages/Cargo.guest.lock;
          };
          cargoConfig = ''
            [source.crates-io]
            replace-with = "vendored-sources"
            [source.vendored-sources]
            directory = "${guestVendor}"
          '';
        in pkgs.runCommand "d2b-guest-rust-deny" {
          nativeBuildInputs = [
            pkgs.cargo-deny
            rustToolchain.cargo
            rustToolchain.rustc
          ];
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
          if !policyInputsPresent then
            pkgs.runCommand "d2b-guest-shell-runner-static-dependency-policy-input" { } ''
              printf '%s\n' D2B-BZLPOLICY-INPUT >&2
              exit 1
            ''
          else
            pkgs.runCommand "d2b-guest-shell-runner-static-dependency-policy" {
              nativeBuildInputs = [ pkgs.gnugrep ];
            } ''
              set -euo pipefail
              fail() {
                printf '%s\n' "$1" >&2
                exit 1
              }
              closure=${guestPolicyRoot}/production/closure.json
              lock=${guestPolicyRoot}/production/Cargo.lock
              test -s "$closure" || fail D2B-BZLPOLICY-INPUT
              test -s "$lock" || fail D2B-BZLPOLICY-INPUT
              if grep -E 'name = "(openssl|openssl-sys|native-tls|libsystemd|systemd|pam-sys|dlopen2)"' "$lock"; then
                fail D2B-BZLPOLICY-CLOSURE
              fi
              mkdir -p "$out"
              printf '%s\n' ok > "$out/result"
            '';

        broker-production-dependency-policy =
          mkPolicyInputCheck {
            name = "broker-production-dependency-policy";
            root = brokerPolicyRoot;
            package = "d2b-priv-broker";
            target = nativeGnuTarget;
            feature = "";
            variant = "production";
            edgeKinds = "normal,build";
            production = true;
          };

        guest-real-libshpool-package-policy =
          mkPolicyInputCheck {
            name = "guest-real-libshpool-package-policy";
            root = guestPolicyRoot;
            package = "d2b-guest-shell-runner";
            target = nativeMuslTarget;
            feature = "real-libshpool";
            variant = "policy";
            edgeKinds = "normal,build,dev";
            production = false;
          };

        broker-production-package-policy =
          mkPolicyInputCheck {
            name = "broker-production-package-policy";
            root = brokerPolicyRoot;
            package = "d2b-priv-broker";
            target = nativeGnuTarget;
            feature = "";
            variant = "policy";
            edgeKinds = "normal,build,dev";
            production = false;
          };

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
      });

      overlays.default = _final: _prev: { };
    };
}
