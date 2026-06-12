{
  description = "Opinionated NixOS desktop microVM workspaces";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # `microvm` flake input DROPPED per ADR 0018.
    # The nixling NixOS substrate owns its per-VM evaluator via
    # `nixos-modules/vm-evaluator.nix` + `nixos-modules/vm-options.nix`.
    # Runner argv generation lives in the Rust crate
    # `packages/nixling-host/src/*_argv.rs` (broker-side).

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
      #   imports = [ inputs.nixling.nixosModules.default ];
      #
      # Future work will populate the remaining surface:
      #   packages.<sys>       — patched cloud-hypervisor, crosvm, etc.
      #   apps.<sys>           — the `nixling` CLI as a runnable app
      #   templates.default    — `nix flake init -t github:vicondoa/nixling`
      #   checks.<sys>         — flake-eval CI gates
      #   lib                  — re-exported helpers (subnetIp, mkMac, …)
      #   overlays.default     — adds vhostDeviceSound, crosvmPatched, …
      nixosModules.default = import ./nixos-modules { inherit inputs; };

      packages = forAllSystems (system: let
        pkgs = nixpkgsFor.${system};
        guestRustPackagesSrc = pkgs.runCommand "nixling-guest-rust-src" { } ''
          mkdir -p $out/packages
          cp -r ${./packages/nixling-core} $out/packages/nixling-core
          cp -r ${./packages/nixling-ipc} $out/packages/nixling-ipc
          cp -r ${./packages/nixling-guestd} $out/packages/nixling-guestd
          cp -r ${./packages/nixling-userd} $out/packages/nixling-userd
          cp -r ${./packages/nixling-exec-runner} $out/packages/nixling-exec-runner
          cp ${./packages/Cargo.guest.lock} $out/packages/Cargo.lock
          chmod -R u+w $out/packages/nixling-core
          cat > $out/packages/nixling-core/Cargo.toml <<'EOF'
          [package]
          name = "nixling-core"
          version = "0.0.0-bootstrap"
          edition = "2021"
          publish = false
          license.workspace = true

          [lib]
          test = false
          doctest = false

          [lints]
          workspace = true

          [features]
          test-support = []

          [dependencies]
          serde.workspace = true
          serde_json.workspace = true
          schemars.workspace = true
          semver = "1"
          rustix = { workspace = true }
          sha2 = { workspace = true }
          EOF
          cat > $out/packages/Cargo.toml <<'EOF'
          [workspace]
          resolver = "2"
          members = [
            "nixling-core",
            "nixling-ipc",
            "nixling-guestd",
            "nixling-userd",
            "nixling-exec-runner",
          ]

          [workspace.package]
          license = "Apache-2.0"

          [workspace.lints.clippy]
          all = "warn"

          [workspace.lints.rust]
          unsafe_code = "forbid"
          unexpected_cfgs = { level = "warn", check-cfg = ["cfg(test_root)"] }

          [workspace.dependencies]
          serde = { version = "1", features = ["derive"] }
          serde_json = "1"
          schemars = { version = "0.8", features = ["derive"] }
          rustix = { version = "0.38", features = ["fs", "process", "net", "pipe", "system"] }
          sha2 = "0.10"
          EOF
        '';
        cargoLock = {
          lockFile = ./packages/Cargo.guest.lock;
        };
        guestStaticPackage = packageName: binName:
          pkgs.pkgsStatic.rustPlatform.buildRustPackage {
            pname = "${binName}-static";
            version = "0.0.0-bootstrap";
            src = guestRustPackagesSrc;
            sourceRoot = "nixling-guest-rust-src/packages";
            inherit cargoLock;
            cargoBuildFlags = [ "--package" packageName "--bin" binName ];
            doCheck = false;
            RUSTC_WRAPPER = "";
            SCCACHE_DIR = "";
            nativeBuildInputs = [ pkgs.binutils ];
            postInstall = ''
              bin="$out/bin/${binName}"
              test -x "$bin"
              readelf -h "$bin" >/dev/null
              readelf -l "$bin" > "$TMPDIR/${binName}.program-headers"
              if grep -q 'Requesting program interpreter' "$TMPDIR/${binName}.program-headers"; then
                echo "${binName}: unexpected ELF interpreter" >&2
                cat "$TMPDIR/${binName}.program-headers" >&2
                exit 1
              fi
              if readelf -d "$bin" > "$TMPDIR/${binName}.dynamic" 2> "$TMPDIR/${binName}.dynamic.err"; then
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
      in {
        manpages = pkgs.runCommand "nixling-manpages" { } ''
          install -Dm644 ${./docs/manpages/nixling.1} "$out/share/man/man1/nixling.1"
          ${pkgs.gzip}/bin/gzip -n -c ${./docs/manpages/nixling.1} > "$out/share/man/man1/nixling.1.gz"
        '';

        completions = pkgs.runCommand "nixling-completions" { } ''
          install -Dm644 ${./docs/completions/nixling.bash} "$out/share/bash-completion/completions/nixling"
          install -Dm644 ${./docs/completions/nixling.zsh}  "$out/share/zsh/site-functions/_nixling"
          install -Dm644 ${./docs/completions/nixling.fish} "$out/share/fish/vendor_completions.d/nixling.fish"
        '';
        nixling-guestd-static = guestStaticPackage "nixling-guestd" "nixling-guestd";
        nixling-userd-static = guestStaticPackage "nixling-userd" "nixling-userd";
        nixling-exec-runner-static =
          guestStaticPackage "nixling-exec-runner" "nixling-exec-runner";

        signoz = import ./pkgs/signoz { inherit pkgs; };
        signozOtelCollector = import ./pkgs/signoz-otel-collector { inherit pkgs; };
        signozSchemaMigrator = import ./pkgs/signoz-schema-migrator { inherit pkgs; };
      });

      apps = forAllSystems (system: { });

      templates.default = {
        path = ./templates/default;
        description = "Minimal nixling host scaffold — one env, one headless workload VM";
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
        nixlingModule = import ./nixos-modules { inherit inputs; };
        mkEval = modules: nixpkgs.lib.nixosSystem {
          inherit system;
          modules = [ nixlingModule ] ++ modules;
        };
        mkCheck = name: cfg: pkgs.runCommand "nixling-check-${name}" { } ''
          echo ${builtins.unsafeDiscardStringContext cfg.config.system.build.toplevel.drvPath} > $out
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
        rustPackagesSrc = pkgs.runCommand "nixling-rust-src" { } ''
          mkdir -p $out/packages
          cp -r ${./packages}/. $out/packages/
          mkdir -p $out/tests
          cp -r ${./tests/golden} $out/tests/golden
          cp -r ${./tests/fixtures} $out/tests/fixtures
        '';
        guestRustPackagesSrc = pkgs.runCommand "nixling-guest-rust-src" { } ''
          mkdir -p $out/packages
          cp -r ${./packages/nixling-core} $out/packages/nixling-core
          cp -r ${./packages/nixling-ipc} $out/packages/nixling-ipc
          cp -r ${./packages/nixling-guestd} $out/packages/nixling-guestd
          cp -r ${./packages/nixling-userd} $out/packages/nixling-userd
          cp -r ${./packages/nixling-exec-runner} $out/packages/nixling-exec-runner
          cp ${./packages/Cargo.guest.lock} $out/packages/Cargo.lock
          chmod -R u+w $out/packages/nixling-core
          cat > $out/packages/nixling-core/Cargo.toml <<'EOF'
          [package]
          name = "nixling-core"
          version = "0.0.0-bootstrap"
          edition = "2021"
          publish = false
          license.workspace = true

          [lib]
          test = false
          doctest = false

          [lints]
          workspace = true

          [features]
          test-support = []

          [dependencies]
          serde.workspace = true
          serde_json.workspace = true
          schemars.workspace = true
          semver = "1"
          rustix = { workspace = true }
          sha2 = { workspace = true }
          EOF
          cat > $out/packages/Cargo.toml <<'EOF'
          [workspace]
          resolver = "2"
          members = [
            "nixling-core",
            "nixling-ipc",
            "nixling-guestd",
            "nixling-userd",
            "nixling-exec-runner",
          ]

          [workspace.package]
          license = "Apache-2.0"

          [workspace.lints.clippy]
          all = "warn"

          [workspace.lints.rust]
          unsafe_code = "forbid"
          unexpected_cfgs = { level = "warn", check-cfg = ["cfg(test_root)"] }

          [workspace.dependencies]
          serde = { version = "1", features = ["derive"] }
          serde_json = "1"
          schemars = { version = "0.8", features = ["derive"] }
          rustix = { version = "0.38", features = ["fs", "process", "net", "pipe", "system"] }
          sha2 = "0.10"
          EOF
        '';
        rustWorkspace = args: pkgs.rustPlatform.buildRustPackage ({
          pname = "nixling-rust-workspace";
          version = "0.0.0-bootstrap";
          src = rustPackagesSrc;
          sourceRoot = "nixling-rust-src/packages";
          cargoLock = {
            lockFile = ./packages/Cargo.lock;
            outputHashes."wl-proxy-0.1.2" = "sha256-5hnfZksxKQIWVEKYnqwyJGWKrBX1FOMGG+3k/FASoBg=";
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
        brokerManifestToml = builtins.fromTOML (builtins.readFile ./packages/nixling-priv-broker/Cargo.toml);
        mainManifestToml = builtins.fromTOML (builtins.readFile ./packages/Cargo.toml);
        assertRustToolchain = ''
          rustc --version | grep -F "${rustToolchainChannel}"
        '';
        assertRustSupplyChainInputs = ''
          test -f ${rustPackagesSrc}/packages/Cargo.lock
          test -f ${rustPackagesSrc}/packages/Cargo.guest.lock
          test -f ${rustPackagesSrc}/packages/deny.toml
          test -f ${rustPackagesSrc}/packages/nixling-priv-broker/Cargo.lock
          test -f ${rustPackagesSrc}/packages/nixling-priv-broker/deny.toml
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
      in {
        eval-minimal = mkCheck "eval-minimal"
          (mkEval [ (import ./examples/minimal/configuration.nix) ]);

        eval-multi-env = mkCheck "eval-multi-env"
          (mkEval [ (import ./examples/multi-env/configuration.nix) ]);

        rust-build = rustWorkspace {
          pname = "nixling-rust-build";
          preBuild = assertRustToolchain;
          cargoBuildFlags = [ "--workspace" ];
          doCheck = false;
        };

        rust-tests = rustWorkspace {
          pname = "nixling-rust-tests";
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
          pname = "nixling-rust-clippy";
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

        guest-static-elf = pkgs.runCommand "nixling-guest-static-elf" {
          nativeBuildInputs = [ pkgs.binutils ];
        } ''
          for bin in \
            ${self.packages.${system}.nixling-guestd-static}/bin/nixling-guestd \
            ${self.packages.${system}.nixling-userd-static}/bin/nixling-userd \
            ${self.packages.${system}.nixling-exec-runner-static}/bin/nixling-exec-runner
          do
            test -x "$bin"
            name="$(basename "$bin")"
            readelf -h "$bin" >/dev/null
            readelf -l "$bin" > "$TMPDIR/$name.program-headers"
            if grep -q 'Requesting program interpreter' "$TMPDIR/$name.program-headers"; then
              echo "$bin: unexpected ELF interpreter" >&2
              cat "$TMPDIR/$name.program-headers" >&2
              exit 1
            fi
            if readelf -d "$bin" > "$TMPDIR/$name.dynamic" 2> "$TMPDIR/$name.dynamic.err"; then
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
          evidence = import ./tests/guest-static-consumption-eval.nix {
            inherit system pkgs;
            flake = self;
          };
        in pkgs.runCommand "nixling-guest-static-consumption" { } ''
          mkdir -p "$out"
          printf '%s\n' '${evidence}' > "$out/guest-static-consumption.json"
        '';

        guest-exec-policy = let
          evidence = import ./tests/guest-exec-policy-eval.nix {
            inherit system pkgs;
            flake = self;
            scenario = "enabled";
          };
        in pkgs.runCommand "nixling-guest-exec-policy" { } ''
          mkdir -p "$out"
          printf '%s\n' '${evidence}' > "$out/guest-exec-policy.json"
        '';

        guest-control-vsock = let
          evidence = import ./tests/guest-control-vsock-eval.nix {
            inherit system pkgs;
            flake = self;
            scenario = "base";
          };
        in pkgs.runCommand "nixling-guest-control-vsock" { } ''
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
            outputHashes."wl-proxy-0.1.2" = "sha256-5hnfZksxKQIWVEKYnqwyJGWKrBX1FOMGG+3k/FASoBg=";
          };
          brokerVendor = pkgs.rustPlatform.importCargoLock {
            lockFile = ./packages/nixling-priv-broker/Cargo.lock;
          };
          cargoConfig = vendorDir: ''
            [source.crates-io]
            replace-with = "vendored-sources"
            [source."git+https://github.com/vicondoa/wl-proxy.git?rev=83b0001ce6c1f8d379609b07b7bcb8528bd044cd#83b0001ce6c1f8d379609b07b7bcb8528bd044cd"]
            git = "https://github.com/vicondoa/wl-proxy.git"
            rev = "83b0001ce6c1f8d379609b07b7bcb8528bd044cd"
            replace-with = "vendored-sources"
            [source.vendored-sources]
            directory = "${vendorDir}"
          '';
        in pkgs.runCommand "nixling-rust-deny" {
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
            "nixling-priv-broker/Cargo.toml" \
            '${cargoConfig brokerVendor}' \
            "${rustPackagesSrc}/packages/nixling-priv-broker/deny.toml"

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
        in pkgs.runCommand "nixling-guest-rust-deny" {
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
        rust-audit = pkgs.runCommand "nixling-rust-audit" {
          nativeBuildInputs = [ pkgs.cargo-audit ];
        } ''
          export HOME="$TMPDIR"
          for lock in \
            ${rustPackagesSrc}/packages/Cargo.lock \
            ${rustPackagesSrc}/packages/Cargo.guest.lock \
            ${rustPackagesSrc}/packages/nixling-priv-broker/Cargo.lock; do
            echo "==> cargo audit ($(basename "$(dirname "$lock")"))"
            cargo-audit audit --file "$lock" \
              --db ${advisoryDbGit} --no-fetch
          done
          echo ok > $out
        '';

        guest-static-dependency-policy =
          pkgs.runCommand "nixling-guest-static-dependency-policy" { } ''
            lock=${./packages/Cargo.guest.lock}
            if grep -E 'name = "(cc|cmake|pkg-config|openssl|openssl-sys|native-tls|libsystemd|systemd)"' "$lock"; then
              echo "guest static lock contains a native-link/build-script dependency" >&2
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
            nixling.site.launcherUsers = lib.mkForce [ "check-user" ];
            nixling.site.userAuthorizedKeys = lib.mkForce [
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

      lib = nixpkgs.lib.makeExtensible (_: { });

      overlays.default = _final: _prev: { };
    };
}
