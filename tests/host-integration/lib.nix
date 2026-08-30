# Shared node configuration for d2b runNixOSTest (type-G) integration
# tests. These are the additive, real-kernel coverage layer: a runNixOSTest VM
# boots a real NixOS system with the d2b daemon surface
# (`d2b.daemonExperimental.enable`) and the test script asserts live broker
# / daemon behaviour (socket activation, SO_PEERCRED, the public.sock wire
# surface, audited host mutations) that the PR-tier fake-backed Rust canaries
# and pure-eval gates cannot exercise.
#
# This file is NOT a flake check: the VM tests live under the `vmChecks` flake
# output (selected explicitly by `make test-host-integration`), so the Layer-1
# `nix flake check --no-build --all-systems` never realizes a VM.
{ self, lib }:

let
  # The minimal, hermetic d2b site declaration every daemon-host node shares.
  # Zone and Guest resources belong to the acceptance fixture that exercises
  # them; keeping this base free of legacy VM/env authoring prevents unrelated
  # host checks from silently materializing a second lifecycle graph.
  daemonAcceptanceUnits = [
    "d2bd.service"
    "d2b-broker.socket"
    "d2b-broker.service"
  ];

  baseD2bConfig = {
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
      usePrebuiltHostTools = false;
    };
    # The full daemon + broker systemd surface under test.
    d2b.daemonExperimental.enable = true;
  };

  mkGuestSystem =
    { pkgs, name, zone ? "work", modules ? [ ] }:
    self.lib.evalGuest {
      system = pkgs.stdenv.hostPlatform.system;
      inherit name zone;
      modules = [
        ({ lib, ... }: {
          boot.loader.grub.enable = lib.mkForce false;
          boot.loader.systemd-boot.enable = lib.mkForce false;
          boot.initrd.includeDefaultModules = false;
          fileSystems."/" = {
            device = "tmpfs";
            fsType = "tmpfs";
          };
          environment.etc."machine-id".text =
            "00000000000000000000000000000000";
          networking.hostName = name;
          users.users.alice = {
            isNormalUser = true;
            uid = 1000;
          };
          system.stateVersion = "25.11";
        })
      ] ++ modules;
    };

  mkRuntimeCloudHypervisorArtifact = pkgs:
    let
      rustPackagesSrc = pkgs.runCommand "d2b-u20-rust-src" { } ''
        mkdir -p "$out/packages"
        cp ${../../Cargo.toml} "$out/Cargo.toml"
        cp ${../../Cargo.lock} "$out/Cargo.lock"
        cp ${../../deny.toml} "$out/deny.toml"
        cp -r ${../../packages}/. "$out/packages/"
        mkdir -p "$out/docs/reference/schemas/v3/providers"
        cp ${../../docs/reference/schemas/v3/providers/transport-azure-relay.transport-settings.json} \
          "$out/docs/reference/schemas/v3/providers/transport-azure-relay.transport-settings.json"
        cp ${../../docs/reference/schemas/v3/providers/transport-vsock.transport-binding.json} \
          "$out/docs/reference/schemas/v3/providers/transport-vsock.transport-binding.json"
      '';
      controller = pkgs.rustPlatform.buildRustPackage {
        pname = "d2b-u20-cloud-hypervisor-controller";
        version = "0.0.0";
        src = rustPackagesSrc;
        sourceRoot = "d2b-u20-rust-src";
        cargoLock = {
          lockFile = ../../Cargo.lock;
          outputHashes."wl-proxy-0.1.2" =
            "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
        };
        cargoBuildFlags = [
          "--package"
          "d2b-provider-runtime-cloud-hypervisor"
          "--bin"
          "d2b-cloud-hypervisor-controller"
        ];
        doCheck = false;
        RUSTC_WRAPPER = "";
        SCCACHE_DIR = "";
        installPhase = ''
          runHook preInstall
          binary=$(find target -type f \
            -name d2b-cloud-hypervisor-controller -print -quit)
          if [ -z "$binary" ]; then
            find target -maxdepth 4 -type f -print >&2
            exit 1
          fi
          install -Dm755 "$binary" \
            "$out/bin/d2b-cloud-hypervisor-controller"
          runHook postInstall
        '';
      };
      signer = pkgs.python3.withPackages
        (pythonPackages: [ pythonPackages.cryptography ]);
      manifest = ../../packages/d2b-provider-runtime-cloud-hypervisor/provider-manifest.json;
      schema = ../../packages/d2b-provider-runtime-cloud-hypervisor/root-config.schema.json;
      package = pkgs.runCommand "d2b-u20-runtime-cloud-hypervisor" {
        nativeBuildInputs = [ pkgs.coreutils signer ];
      } ''
        ${signer}/bin/python3 - "${manifest}" \
          "${controller}/bin/d2b-cloud-hypervisor-controller" "$out" <<'PY'
        import hashlib
        import json
        import pathlib
        import sys
        from cryptography.hazmat.primitives import serialization
        from cryptography.hazmat.primitives.asymmetric.ed25519 import (
            Ed25519PrivateKey,
        )

        manifest_path, controller_path, output_path = sys.argv[1:]
        manifest = json.loads(pathlib.Path(manifest_path).read_text())
        raw_digest = "sha256:" + hashlib.sha256(
            pathlib.Path(controller_path).read_bytes()
        ).hexdigest()
        executable_map = json.dumps(
            {"d2b-cloud-hypervisor-controller": raw_digest},
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode()
        first = hashlib.sha256(
            b"d2b:v3:provider-executable-set\0" + executable_map
        ).digest()
        executable_digest = "sha256:" + hashlib.sha256(first).hexdigest()
        manifest["digests"]["executable"] = executable_digest
        manifest_bytes = json.dumps(
            manifest,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode()
        seed = hashlib.sha256(
            b"d2b-u20-runtime-cloud-hypervisor-signing-key-v1"
            + raw_digest.encode()
        ).digest()
        private_key = Ed25519PrivateKey.from_private_bytes(seed)
        public_key = private_key.public_key().public_bytes(
            serialization.Encoding.PEM,
            serialization.PublicFormat.SubjectPublicKeyInfo,
        )
        signature = private_key.sign(manifest_bytes)
        output = pathlib.Path(output_path)
        (output / "bin").mkdir(parents=True)
        (output / "share/d2b/provider").mkdir(parents=True)
        controller_output = output / "bin/d2b-cloud-hypervisor-controller"
        controller_output.write_bytes(pathlib.Path(controller_path).read_bytes())
        controller_output.chmod(0o755)
        (output / "share/d2b/provider/provider-manifest.json").write_bytes(
            manifest_bytes
        )
        (output / "share/d2b/provider/provider-manifest.json.sig").write_bytes(
            signature
        )
        (output / "share/d2b/provider/config-schema.json").write_bytes(
            pathlib.Path("${schema}").read_bytes()
        )
        (output / "publisher-public-key.pem").write_bytes(public_key)
        (output / "executable-set-digest").write_text(executable_digest)
        (output / "manifest-digest").write_text(
            "sha256:" + hashlib.sha256(manifest_bytes).hexdigest()
        )
        PY
      '';
      packageDigestPath = pkgs.runCommand
        "d2b-u20-runtime-cloud-hypervisor-nar-digest" {
          nativeBuildInputs = [ pkgs.nix ];
        } ''
          printf 'sha256:%s' \
            "$(${pkgs.nix}/bin/nix --extra-experimental-features nix-command \
              hash path --type sha256 --base16 "${package}")" > "$out"
        '';
      manifestData = builtins.fromJSON (builtins.readFile manifest);
      component = lib.head manifestData.components;
      apiBinding = lib.head manifestData.apiBindings;
      runtime = lib.head manifestData.runtimeArtifacts;
      catalog = {
        providerName = manifestData.trust.publisher;
        packageName = "d2b-provider-runtime-cloud-hypervisor";
        version = "0.0.0";
        systems = [ pkgs.stdenv.hostPlatform.system ];
        platform = pkgs.stdenv.hostPlatform.system;
        apiCompatibility = "d2b.zone.v3";
        serviceCompatibility = "d2bd.resource";
        signature = { signatureId = "default"; };
        rootEpoch = manifestData.trust.rootEpoch;
        revocationStatus = manifestData.trust.revocation;
        denyStatus = "clear";
        provenanceEvidence = manifestData.trust.provenance;
        sbomEvidence = manifestData.trust.sbom;
        licenseEvidence = manifestData.trust.license;
        vulnerabilityEvidence = manifestData.trust.vulnerability;
        conformanceAttestation = manifestData.trust.conformance;
        supportChannel = manifestData.trust.supportChannel;
        supportContact = "d2b-acceptance@localhost";
        publisher = manifestData.trust.publisher;
        packageDigest = lib.removeSuffix "\n"
          (builtins.readFile packageDigestPath);
        executableDigest = lib.removeSuffix "\n"
          (builtins.readFile "${package}/executable-set-digest");
        manifestDigest = lib.removeSuffix "\n"
          (builtins.readFile "${package}/manifest-digest");
        componentDigest = "sha256:${builtins.hashString
          "sha256" (builtins.toJSON manifestData.components)}";
        descriptorDigest = "sha256:${builtins.hashString
          "sha256" (builtins.toJSON manifestData.apiBindings)}";
        configDigest = manifestData.digests.config;
        d2bdDigest = runtime.d2bdDigest;
        brokerDigest = runtime.brokerDigest;
        instanceScope = component.instanceScope;
        supportedTargetKinds = component.supportedTargetKinds;
        targetCapabilities = component.targetCapabilities;
        placementAnchor = apiBinding.placementAnchor;
      };
    in {
      inherit package catalog;
      type = "provider";
      trustedPublisher = {
        publisherRef = manifestData.trust.publisher;
        signingKey = builtins.readFile "${package}/publisher-public-key.pem";
      };
    };

  mkVolumeProviderArtifact = pkgs:
    let
      manifest = ../../tests/fixtures/provider-volume-acceptance/provider-manifest.json;
      schema = ../../tests/fixtures/provider-volume-acceptance/config-schema.json;
      signature = ../../tests/fixtures/provider-volume-acceptance/provider-manifest.json.sig;
      publicKey = ../../tests/fixtures/provider-volume-acceptance/publisher-public-key.pem;
      package = pkgs.runCommand "d2b-volume-acceptance-provider" {
        nativeBuildInputs = [ pkgs.coreutils ];
      } ''
        install -Dm755 ${pkgs.coreutils}/bin/coreutils \
          "$out/bin/acceptance-controller"
        install -Dm644 ${manifest} \
          "$out/share/d2b/provider/provider-manifest.json"
        install -Dm644 ${signature} \
          "$out/share/d2b/provider/provider-manifest.json.sig"
        install -Dm644 ${schema} \
          "$out/share/d2b/provider/config-schema.json"
      '';
      packageDigestPath = pkgs.runCommand
        "d2b-volume-acceptance-provider-nar-digest" {
          nativeBuildInputs = [ pkgs.nix ];
        } ''
          printf 'sha256:%s' \
            "$(${pkgs.nix}/bin/nix --extra-experimental-features nix-command \
              hash path --type sha256 --base16 "${package}")" > "$out"
        '';
      catalog = {
        providerName = "volume-acceptance-provider";
        packageName = "d2b-volume-acceptance-provider";
        version = "0.0.0";
        systems = [ "x86_64-linux" ];
        platform = "x86_64-linux";
        apiCompatibility = "d2b.zone.v3";
        serviceCompatibility = "d2bd.resource";
        signature = { signatureId = "default"; };
        rootEpoch = 1;
        revocationStatus = "clear";
        denyStatus = "clear";
        provenanceEvidence = "accepted";
        sbomEvidence = "accepted";
        licenseEvidence = "accepted";
        vulnerabilityEvidence = "accepted";
        conformanceAttestation = "accepted";
        supportChannel = "stable";
        supportContact = "d2b-acceptance@localhost";
        publisher = "d2b-volume-acceptance";
        packageDigest = lib.removeSuffix "\n"
          (builtins.readFile packageDigestPath);
        executableDigest = "sha256:f84125779653dba770042fd2af2bd01299b05ae892c039c497e6b5ce45029d9c";
        manifestDigest = "sha256:74d93ded3147181a610b094772fe13e13c226b9986108771cc768408a4d1b5f5";
        componentDigest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        descriptorDigest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        configDigest = "sha256:ccb5a9d66e068ea8f4e205788589675a48e9e3754a840d8ac10120d14238e914";
      };
    in {
      inherit package catalog;
      type = "provider";
      trustedPublisher = {
        publisherRef = "d2b-volume-acceptance";
        signingKey = builtins.readFile publicKey;
      };
    };
in
{
  # A NixOS module for a runNixOSTest node that boots the d2b daemon host.
  # `extra` is merged as an additional module so individual tests can add
  # per-test Zone/Guest resources, tampering helpers, or a larger disk. The
  # node provisions the `alice` operator user the base config references.
  #
  # Structured as an attrset-module with everything in `imports` (an attrset is
  # a valid module): `imports` must be top-level, NOT wrapped in `lib.mkMerge`,
  # or the module system rejects it ("option nodes.machine.imports does not
  # exist").
  d2bDaemonNode =
    { extra ? { }, writableStore ? false }:
    { config, ... }:
    {
      imports = [
        self.nixosModules.default
        baseD2bConfig
        extra
        {
          # Headroom for building/activating the bundle + daemon closure inside
          # the VM; the default 1024 MiB is tight once the broker spawns
          # runners.
          virtualisation.memorySize = 3072;
          virtualisation.diskSize = 8192;

          users.users.alice = {
            isNormalUser = true;
            uid = 1000;
          };

          environment.etc."d2b/daemon-acceptance-units".text =
            lib.concatStringsSep "\n" daemonAcceptanceUnits + "\n";

          # runNixOSTest runs first-boot activation before systemd-tmpfiles has
          # materialized the d2b state tree. Pre-create the state directory so
          # daemon-owned startup can rely on the same path ordering.
          system.activationScripts.d2bTestStateDirs = {
            deps = [ "users" ];
            text = ''
              install -d -m 0750 -o root -g d2bd /var/lib/d2b
              install -d -m 0710 -o root -g d2b /var/lib/d2b/keys
              : > /var/lib/d2b/keys/.lock
              chown root:root /var/lib/d2b/keys/.lock
              chmod 0600 /var/lib/d2b/keys/.lock
            '';
          };
          system.stateVersion = "25.11";
        }
        # Opt-in writable same-fs store. ONLY needed by tests that drive the
        # per-VM /nix/store hardlink farm (which requires /var/lib/d2b and
        # /nix/store on the SAME filesystem - hardlinks can't cross FS - and the
        # default runNixOSTest read-only store image splits them). It is OFF by
        # default: `virtualisation.writableStore = true` copies the entire guest
        # closure into a writable overlay at boot, which adds many minutes to
        # (and can hang) VM startup. The daemon/broker activation + host-posture
        # tests (daemon-smoke, bridge-isolation, state-dir-acl, privilege-oracle)
        # never boot a microVM, so they never touch the farm - keep this off for
        # a fast, reliable boot.
        (lib.mkIf writableStore {
          virtualisation.writableStore = true;
        })
      ];
    };

  # Re-exported so tests can assert against the shared declaration.
  inherit baseD2bConfig mkGuestSystem mkRuntimeCloudHypervisorArtifact
    mkVolumeProviderArtifact;
}
