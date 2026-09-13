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
{ self, lib, hostToolBundle ? null }:

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
    # The daemon's v3 bundle always carries the local-root storage row. Keep
    # the corresponding root Zone in these minimal host fixtures so the
    # emitted topology is sealed and the daemon can enter Ready.
    d2b.zones.local-root = { };
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
      controller =
        self.packages.${pkgs.stdenv.hostPlatform.system}.d2b-cloud-hypervisor-controller;
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
    { config, pkgs, ... }:
    let
      # Dedicated state disk: the redb Zone store pays an fsync per commit
      # on the emulated root disk, and bring-up write bursts stall the
      # daemon writer thread ~700-900ms per write. Attaching /var/lib/d2b
      # as its own virtio drive with cache=unsafe makes guest fsync a host
      # page-cache no-op; the fixture VM is ephemeral, so the durability
      # semantics that unsafe drops are irrelevant here.
      stateDisk = pkgs.runCommand "d2b-state.img"
        {
          nativeBuildInputs = [ pkgs.e2fsprogs ];
        }
        ''
          truncate -s 4G "$out"
          mkfs.ext4 -q -F "$out"
        '';
    in
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
          # The daemon's redb writer thread, the daemon async runtime, the
          # broker, and three controllers all need real CPU. The 1-vCPU
          # default serializes them and starves every 250ms handshake.
          virtualisation.cores = 3;
          boot.kernelModules = [ "br_netfilter" "tun" "vhost_net" ];

          users.users.alice = {
            isNormalUser = true;
            uid = 1000;
          };

          environment.etc."d2b/daemon-acceptance-units".text =
            lib.concatStringsSep "\n" daemonAcceptanceUnits + "\n";

          # Fail VM checks promptly when daemon startup is deterministically
          # broken instead of spending the lane timeout in a restart loop.
          systemd.services.d2bd.unitConfig = {
            StartLimitIntervalSec = "30s";
            StartLimitBurst = 3;
          };

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
        # tests (daemon-smoke, bridge-isolation, privilege-oracle)
        # never boot a microVM, so they never touch the farm - keep this off for
        # a fast, reliable boot.
        (lib.mkIf writableStore {
          virtualisation.useBootLoader = true;
          # The guest store-view hardlinks /nix/store into /var/lib/d2b, so
          # both must stay on one filesystem - a separate state disk would
          # break the hardlink farm with EXDEV. Instead, drop the root
          # drive's cache to unsafe: every redb commit's fsync becomes a
          # host page-cache no-op instead of a ~700-900ms stall, and the
          # fixture VM is ephemeral, so the lost durability is irrelevant.
          virtualisation.qemu.drives = lib.mkForce [
            {
              name = "root";
              file = ''"$NIX_DISK_IMAGE"'';
              driveExtraOpts.cache = "unsafe";
              driveExtraOpts.werror = "report";
              deviceExtraOpts.bootindex = "1";
              deviceExtraOpts.serial = "root";
            }
          ];
        })
        # The state disk keeps /var/lib/d2b off the emulated root disk:
        # cache=unsafe (host fsync no-op), noatime + nobarrier mounts.
        # The writableStore hardlink-farm tests stay on the default
        # same-fs layout, so this is opt-out for them.
        (lib.mkIf (! writableStore) {
          # The image is a `pkgs.runCommand` output, so QEMU must not need
          # write access to it: the lane builds these checks inside the Nix
          # sandbox, where /nix/store is mounted read-only, and a writable
          # drive on a store path makes QEMU abort at machine start - the
          # test driver surfaces that as a bare "Connection reset by peer".
          # `snapshot=on` opens the backing file read-only and keeps every
          # guest write in an ephemeral per-VM overlay under TMPDIR, which
          # matches the fixture's ephemeral state disk either way.
          virtualisation.qemu.options = [
            "-drive"
            "file=${stateDisk},format=raw,if=virtio,cache=unsafe,aio=threads,snapshot=on"
          ];
          fileSystems."/var/lib/d2b" = {
            device = "/dev/vdb";
            fsType = "ext4";
            options = [ "noatime" "nobarrier" ];
            # Up before activation so d2bTestStateDirs lands inside the
            # mounted filesystem, not under the covered root mountpoint.
            neededForBoot = true;
          };
        })
      ];
    };

  # Shared host posture for every fixture that boots a Cloud Hypervisor Guest.
  # The hardlink-backed Guest store view requires a writable host store on the
  # same filesystem as /var/lib/d2b.
  d2bCloudHypervisorNode =
    { extra ? { } }:
    d2bDaemonNode {
      inherit extra;
      writableStore = true;
    };

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
  fixtureDiagnostics = ''
      # ---- d2b fixture diagnostics (issue #513) --------------------------
      # The test driver discards machine.execute output and does not re-print
      # the output a timed-out wait_until_succeeds last saw, so a failed lane
      # used to leave only the command text in the log. These helpers push the
      # row set and the daemon explanation lines into the driver log (stdout
      # and stderr of the test driver, that is the lane log).
      #
      # Diagnostics only: no assertion and no timeout is changed here.
      import time as _diag_time

      _diag_t0 = _diag_time.monotonic()
      _diag_stage = "startup"

      def _diag_elapsed():
          return f"{_diag_time.monotonic() - _diag_t0:.1f}s"

      def _diag_print(*lines):
          for line in lines:
              print(line, flush=True)

      def stage(name):
          global _diag_stage
          _diag_stage = name
          _diag_print(f"[d2b] stage={name} t={_diag_elapsed()}")

      def diag(command, label="diagnostic output"):
          try:
              status, output = machine.execute(command, timeout=120)
          except Exception as error:
              _diag_print(
                  f"[d2b] stage={_diag_stage} t={_diag_elapsed()} {label}: "
                  f"diagnostic command failed: {error}"
              )
              return -1
          _diag_print(
              f"[d2b] stage={_diag_stage} t={_diag_elapsed()} {label} "
              f"(exit {status}):"
          )
          _diag_print(command)
          for line in output.rstrip().splitlines():
              _diag_print("    " + line)
          return status

      def _diag_journal(unit, token):
          scope = f"-u {unit} " if unit else ""
          select = f"| grep -F -- {token!r} " if token else ""
          return (
              f"journalctl {scope}--no-pager -o cat -b -n 4000 2>/dev/null "
              f"{select}| tail -n 60 || true"
          )

      def unit_dumps(unit):
          """Row dumps for a systemd unit waiting to become active."""
          return [
              (
                  f"{unit} status",
                  f"systemctl status {unit} --no-pager 2>&1 | tail -n 40 "
                  "|| true",
              ),
          ]

      # Every fixture drives one zone as one linux user through the same
      # public socket, so the composed explanation is available without each
      # stage listing the rows it asserted on: `d2b debug` reads the whole
      # zone and prints the ownership tree, the row that is not settled, and
      # the structured failure behind it.
      _diag_zone = "work"
      _diag_user = "alice"

      def diag_debug_zone(label="zone explanation"):
          """The composed `d2b debug` report, always diagnostic and never
          fatal: a failure that happened before the daemon was reachable must
          still print its own stage rather than a diagnostic error. Bounded,
          because a failure can happen before there is anything to explain."""
          status = diag(
              f"runuser -u {_diag_user} -- env "
              f"D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
              f"timeout 60 d2b --zone {_diag_zone} debug {_diag_zone} 2>&1 "
              f"|| true",
              label,
          )
          return status

      def diag_step(name, action, rows=(), explain=(), wait=None, debug=True):
          stage(name)
          try:
              return action()
          except Exception as error:
              labels = ", ".join(label for label, _ in rows) or "none"
              failing = f" wait={name}" if wait else ""
              _diag_print(
                  f"[d2b] FAIL stage={name} t={_diag_elapsed()}{failing} "
                  f"rows=[{labels}]: {error}"
              )
              if wait:
                  _diag_print(f"[d2b] failing wait: {wait}")
              for label, command in rows:
                  diag(command, f"row dump: {label}")
              for unit, token in explain:
                  detail = f"journal {unit or 'all'}"
                  if token:
                      detail += f" lines matching {token!r}"
                  diag(_diag_journal(unit, token), detail)
              if debug:
                  diag_debug_zone()
              raise

      def diag_unit(name, unit, timeout, debug=True):
          """wait_for_unit with the unit status and journal on timeout."""
          return diag_step(
              name,
              lambda: machine.wait_for_unit(unit, timeout=timeout),
              unit_dumps(unit),
              [(unit, None)],
              debug=debug,
          )

      def diag_wait(name, command, timeout, rows=(), explain=(), debug=True):
          return diag_step(
              name,
              lambda: machine.wait_until_succeeds(command, timeout=timeout),
              rows,
              explain,
              command,
              debug=debug,
          )
  '';

  # Re-exported so tests can assert against the shared declaration.
  inherit baseD2bConfig mkGuestSystem mkRuntimeCloudHypervisorArtifact
    mkAcceptanceProviderArtifact mkVolumeProviderArtifact;
}
