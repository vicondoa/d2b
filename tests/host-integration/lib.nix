# Shared helpers for d2b runNixOSTest (type-G) integration tests. These are
# the additive, real-kernel coverage layer: a runNixOSTest VM boots a real
# NixOS system with the d2b daemon surface (`d2b.daemonExperimental.enable`)
# and the test script asserts live broker / daemon behaviour (socket
# activation, SO_PEERCRED, the public.sock wire surface, audited host
# mutations) that the PR-tier fake-backed Rust canaries and pure-eval gates
# cannot exercise.
#
# What stays here is the part bound to the test driver: the driver-coupled
# diagnostics prelude, the nested guest systems only these fixtures boot, and
# the provider artifacts only these fixtures install. The reusable guest
# configuration the lane also needs moved to
# `nix/test-support/host-integration-node.nix`, so a fixture can be removed
# when its check ports without taking its guest declaration with it.
#
# This file is NOT a flake check: the VM tests live under the `vmChecks` flake
# output (selected explicitly by `make test-host-integration`), so the Layer-1
# `nix flake check --no-build --all-systems` never realizes a VM.
{ self, lib, hostToolBundle ? null }:

let
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
      # The controller arrives through the Bazel-staged bundle that
      # `make test-host-integration` hands to the evaluation
      # (`D2B_CH_CONTROLLER_BUNDLE`). The flake's own package overlay exposes
      # `d2b-cloud-hypervisor-controller` only inside `testSelf`, and this
      # helper is reached with the plain `self`, so resolving it through
      # `self.packages` failed every check that builds this artifact.
      stagedControllerBundle = builtins.getEnv "D2B_CH_CONTROLLER_BUNDLE";
      # The staged bundle holds the Bazel-built binary, which still points at
      # the build environment's interpreter. It has to go through the same
      # fixup the flake's own package applies - patched interpreter, resolved
      # runtime libraries - or the broker cannot start it inside the VM
      # ("Could not start dynamically linked executable").
      controller =
        if stagedControllerBundle != "" then
          (import ../../nix/test-support/bazel-host-tools.nix {
            inherit pkgs;
            rawBundle = null;
            rawCloudHypervisorController = builtins.path {
              path = /. + stagedControllerBundle;
              name = "d2b-staged-cloud-hypervisor-controller";
            };
          }).cloudHypervisorControllerPackage
        else
          self.packages.${pkgs.stdenv.hostPlatform.system}.d2b-cloud-hypervisor-controller;
      controllerBinary = "${controller}/bin/d2b-cloud-hypervisor-controller";
      signer = pkgs.python3.withPackages
        (pythonPackages: [ pythonPackages.cryptography ]);
      manifest = ../../packages/d2b-provider-guest-cloud-hypervisor/provider-manifest.json;
      schema = ../../packages/d2b-provider-guest-cloud-hypervisor/root-config.schema.json;
      package = pkgs.runCommand "d2b-u20-runtime-cloud-hypervisor" {
        nativeBuildInputs = [ pkgs.coreutils signer ];
      } ''
        ${signer}/bin/python3 - "${manifest}" \
          "${controllerBinary}" "$out" <<'PY'
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
        for component in manifest.get("components", []):
            for capability in component.get("targetCapabilities", []):
                capability["artifactDigest"] = raw_digest
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
        (output / "raw-executable-digest").write_text(raw_digest)
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
        packageName = "d2b-provider-guest-cloud-hypervisor";
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
        targetCapabilities = map
          (capability: capability // {
            artifactDigest = lib.removeSuffix "\n"
              (builtins.readFile "${package}/raw-executable-digest");
          })
          component.targetCapabilities;
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

  mkAcceptanceProviderArtifact = pkgs:
    let
      controller = if hostToolBundle == null then
        "${self.packages.${pkgs.stdenv.hostPlatform.system}.d2b-provider-test-controller}/bin/d2b-provider-test-controller"
      else
        "${hostToolBundle}/bin/d2b-provider-test-controller";
      signer = pkgs.python3.withPackages
        (pythonPackages: [ pythonPackages.cryptography ]);
      manifest = ../../tests/fixtures/provider-acceptance/provider-manifest.json;
      schema = ../../tests/fixtures/provider-acceptance/config-schema.json;
      package = pkgs.runCommand "d2b-u20-acceptance-provider" {
        nativeBuildInputs = [ signer ];
      } ''
        mkdir -p "$out/bin"
        cp "${controller}" "$out/bin/acceptance-controller"
        chmod 0755 "$out/bin/acceptance-controller"
        ${signer}/bin/python3 - "${manifest}" "$out" <<'PY'
        import hashlib
        import json
        import pathlib
        import sys
        from cryptography.hazmat.primitives import serialization
        from cryptography.hazmat.primitives.asymmetric.ed25519 import (
            Ed25519PrivateKey,
        )

        manifest_path, output_path = sys.argv[1:]
        manifest = json.loads(pathlib.Path(manifest_path).read_text())
        binary_path = pathlib.Path(output_path) / "bin/acceptance-controller"
        binary = binary_path.read_bytes()
        raw_digest = "sha256:" + hashlib.sha256(binary).hexdigest()
        resource_types = {"Device", "Network"}
        manifest["apiBindings"] = [
            binding
            for binding in manifest.get("apiBindings", [])
            if binding.get("resourceType") in resource_types
        ]
        for component in manifest.get("components", []):
            component["exportedResourceTypes"] = [
                resource_type
                for resource_type in component.get("exportedResourceTypes", [])
                if resource_type in resource_types
            ]
        executable_map = json.dumps(
            {"acceptance-controller": raw_digest},
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode()
        first = hashlib.sha256(
            b"d2b:v3:provider-executable-set\0" + executable_map
        ).digest()
        executable_digest = "sha256:" + hashlib.sha256(first).hexdigest()
        manifest["trust"]["publisher"] = "d2b-u20-acceptance"
        manifest["digests"]["executable"] = executable_digest
        for component in manifest.get("components", []):
            for capability in component.get("targetCapabilities", []):
                capability["artifactDigest"] = raw_digest
        manifest_bytes = json.dumps(
            manifest,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode()
        seed = hashlib.sha256(
            b"d2b-u20-acceptance-provider-signing-key-v1"
            + raw_digest.encode()
        ).digest()
        private_key = Ed25519PrivateKey.from_private_bytes(seed)
        public_key = private_key.public_key().public_bytes(
            serialization.Encoding.PEM,
            serialization.PublicFormat.SubjectPublicKeyInfo,
        )
        output = pathlib.Path(output_path)
        metadata = output / "share/d2b/provider"
        metadata.mkdir(parents=True)
        (metadata / "provider-manifest.json").write_bytes(manifest_bytes)
        (metadata / "provider-manifest.json.sig").write_bytes(
            private_key.sign(manifest_bytes)
        )
        (metadata / "config-schema.json").write_bytes(
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
        "d2b-u20-acceptance-provider-nar-digest" {
          nativeBuildInputs = [ pkgs.nix ];
        } ''
          printf 'sha256:%s' \
            "$(${pkgs.nix}/bin/nix --extra-experimental-features nix-command \
              hash path --type sha256 --base16 "${package}")" > "$out"
        '';
      baseManifest = builtins.fromJSON (builtins.readFile manifest);
    in {
      inherit package;
      type = "provider";
      catalog = {
        providerName = "acceptance-provider";
        packageName = "d2b-u20-acceptance-provider";
        version = "0.0.0";
        systems = [ pkgs.stdenv.hostPlatform.system ];
        platform = pkgs.stdenv.hostPlatform.system;
        apiCompatibility = "d2b.zone.v3";
        serviceCompatibility = "d2bd.resource";
        signature = { signatureId = "default"; };
        rootEpoch = baseManifest.trust.rootEpoch;
        revocationStatus = baseManifest.trust.revocation;
        denyStatus = "clear";
        provenanceEvidence = baseManifest.trust.provenance;
        sbomEvidence = baseManifest.trust.sbom;
        licenseEvidence = baseManifest.trust.license;
        vulnerabilityEvidence = baseManifest.trust.vulnerability;
        conformanceAttestation = baseManifest.trust.conformance;
        supportChannel = baseManifest.trust.supportChannel;
        supportContact = "d2b-acceptance@localhost";
        publisher = "d2b-u20-acceptance";
        packageDigest = lib.removeSuffix "\n"
          (builtins.readFile packageDigestPath);
        executableDigest = lib.removeSuffix "\n"
          (builtins.readFile "${package}/executable-set-digest");
        manifestDigest = lib.removeSuffix "\n"
          (builtins.readFile "${package}/manifest-digest");
        componentDigest = "sha256:${builtins.hashString
          "sha256" (builtins.toJSON baseManifest.components)}";
        descriptorDigest = "sha256:${builtins.hashString
          "sha256" (builtins.toJSON baseManifest.apiBindings)}";
        configDigest = baseManifest.digests.config;
      };
      trustedPublisher = {
        publisherRef = "d2b-u20-acceptance";
        signingKey = builtins.readFile "${package}/publisher-public-key.pem";
      };
    };

  mkVolumeProviderArtifact = pkgs:
    let
      controller = if hostToolBundle == null then
        "${self.packages.${pkgs.stdenv.hostPlatform.system}.d2b-provider-test-controller}/bin/d2b-provider-test-controller"
      else
        "${hostToolBundle}/bin/d2b-provider-test-controller";
      signer = pkgs.python3.withPackages
        (pythonPackages: [ pythonPackages.cryptography ]);
      manifest = ../../tests/fixtures/provider-volume-acceptance/provider-manifest.json;
      schema = ../../tests/fixtures/provider-volume-acceptance/config-schema.json;
      package = pkgs.runCommand "d2b-volume-acceptance-provider" {
        nativeBuildInputs = [ signer ];
      } ''
        mkdir -p "$out/bin"
        cp "${controller}" "$out/bin/acceptance-controller"
        chmod 0755 "$out/bin/acceptance-controller"
        # The binding-owned virtiofsd worker launches from this signed
        # artifact; the digest-pinned binary rides in the executable set.
        cp "${pkgs.virtiofsd}/bin/virtiofsd" "$out/bin/virtiofsd"
        chmod 0755 "$out/bin/virtiofsd"
        ${signer}/bin/python3 - "${manifest}" "$out" <<'PY'
        import hashlib
        import json
        import pathlib
        import sys
        from cryptography.hazmat.primitives import serialization
        from cryptography.hazmat.primitives.asymmetric.ed25519 import (
            Ed25519PrivateKey,
        )

        manifest_path, output_path = sys.argv[1:]
        output = pathlib.Path(output_path)
        manifest = json.loads(pathlib.Path(manifest_path).read_text())
        binary = (output / "bin/acceptance-controller").read_bytes()
        raw_digest = "sha256:" + hashlib.sha256(binary).hexdigest()
        virtiofsd = (output / "bin/virtiofsd").read_bytes()
        virtiofsd_digest = "sha256:" + hashlib.sha256(virtiofsd).hexdigest()
        executable_map = json.dumps(
            {
                "acceptance-controller": raw_digest,
                "virtiofsd": virtiofsd_digest,
            },
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode()
        first = hashlib.sha256(
            b"d2b:v3:provider-executable-set\0" + executable_map
        ).digest()
        executable_digest = "sha256:" + hashlib.sha256(first).hexdigest()
        manifest["digests"]["executable"] = executable_digest
        for component in manifest.get("components", []):
            for capability in component.get("targetCapabilities", []):
                capability["artifactDigest"] = raw_digest
        manifest_bytes = json.dumps(
            manifest,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode()
        seed = hashlib.sha256(
            b"d2b-u20-volume-acceptance-provider-signing-key-v1"
            + raw_digest.encode()
        ).digest()
        private_key = Ed25519PrivateKey.from_private_bytes(seed)
        metadata = output / "share/d2b/provider"
        metadata.mkdir(parents=True)
        (metadata / "provider-manifest.json").write_bytes(manifest_bytes)
        (metadata / "provider-manifest.json.sig").write_bytes(
            private_key.sign(manifest_bytes)
        )
        (metadata / "config-schema.json").write_bytes(
            pathlib.Path("${schema}").read_bytes()
        )
        (output / "publisher-public-key.pem").write_bytes(
            private_key.public_key().public_bytes(
                serialization.Encoding.PEM,
                serialization.PublicFormat.SubjectPublicKeyInfo,
            )
        )
        (output / "executable-set-digest").write_text(executable_digest)
        (output / "manifest-digest").write_text(
            "sha256:" + hashlib.sha256(manifest_bytes).hexdigest()
        )
        PY
      '';
      packageDigestPath = pkgs.runCommand
        "d2b-volume-acceptance-provider-nar-digest" {
          nativeBuildInputs = [ pkgs.nix ];
        } ''
          printf 'sha256:%s' \
            "$(${pkgs.nix}/bin/nix --extra-experimental-features nix-command \
              hash path --type sha256 --base16 "${package}")" > "$out"
        '';
      baseManifest = builtins.fromJSON (builtins.readFile manifest);
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
        executableDigest = lib.removeSuffix "\n"
          (builtins.readFile "${package}/executable-set-digest");
        manifestDigest = lib.removeSuffix "\n"
          (builtins.readFile "${package}/manifest-digest");
        componentDigest = "sha256:${builtins.hashString
          "sha256" (builtins.toJSON baseManifest.components)}";
        descriptorDigest = "sha256:${builtins.hashString
          "sha256" (builtins.toJSON baseManifest.apiBindings)}";
        configDigest = "sha256:ccb5a9d66e068ea8f4e205788589675a48e9e3754a840d8ac10120d14238e914";
      };
    in {
      inherit package catalog;
      type = "provider";
      trustedPublisher = {
        publisherRef = "d2b-volume-acceptance";
        signingKey = builtins.readFile "${package}/publisher-public-key.pem";
      };
    };
in
rec {
  # Fixture diagnostics prelude, interpolated at the top of each VM fixture's
  # `testScript` (issue #513). The runNixOSTest driver discards
  # `machine.execute` output and never re-prints what a timed-out
  # `wait_until_succeeds` last saw, so the row set at failure never reached the
  # lane log. The prelude provides:
  #
  #   stage(name)          announce the phase, so a failure names it in one line
  #   diag(cmd, label)     run a diagnostic command and echo its captured output
  #   diag_step(name, action, rows=..., explain=..., wait=...)
  #                        run one step; on failure echo its dumps and the
  #                        daemon explanation lines
  #   diag_wait(name, command, timeout, rows=..., explain=...)
  #                        the `wait_until_succeeds` form of `diag_step`
  #
  # `rows` entries are (label, shell command) dumps of the row set the stage
  # was asserting on; `explain` entries are (journal unit or null, token) whose
  # last daemon lines explain those rows. Diagnostics only: every assertion and
  # timeout is passed through unchanged.
  #
  # The text itself lives with the lane's own assertion surface, in
  # `packages/d2b-test-vm-harness/src/diagnostics.py`, and is read from there
  # rather than kept here. The Bazel lane runs these very same evaluated
  # scripts, so a check that has not been ported yet reports its failure
  # through this text under either lane; a second copy of it would be a
  # second dialect of the same diagnostics, and the two would drift the first
  # time one of them gained a helper the other did not.
  fixtureDiagnostics =
    builtins.readFile ../../packages/d2b-test-vm-harness/src/diagnostics.py;

  # Re-exported so tests can assert against the shared declaration.
  inherit mkGuestSystem mkRuntimeCloudHypervisorArtifact
    mkAcceptanceProviderArtifact mkVolumeProviderArtifact;
}
