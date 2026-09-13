# Type-G runNixOSTest: the U17 Device-worker launch path (slice 3).
#
# One Device declares the Provider's swtpm rows, one declares the GPU/video
# rows. The fixture proves, on a live host:
#
# 1. TPM (no hardware needed). `Process/swtpm-<device>` and
#    `EphemeralProcess/swtpm-flush-<device>` are the declared rows of the
#    owning Device; the swtpm worker really runs with the argv the Process
#    controller composed (state dir + ctrl/server sockets + principal), binds
#    its sockets, and the one-shot flush publishes its outcome as the row's
#    status projection; deleting the Device retires the declared rows through
#    the Process controller (children first).
# 2. GPU (path only). No GPU exists in the VM, so the fixture pins the launch
#    PATH: the declared `Process/gpu-<device>` rows resolve, their launch is
#    attempted through the Process controller and the broker, and the outcome
#    is a named refusal on the row - never a bare launch, never Ready, and
#    never a fake device.
#
# The worker executables come from the fixture's own signed Provider
# artifacts: `swtpm`/`swtpm-ioctl` are the real binaries (nixpkgs swtpm); the
# GPU artifact's `crosvm` is a fixture stand-in (an ELF shim that records its
# argv and refuses), because the fixture never intends a GPU worker to serve -
# it stands in for the executable a real GPU Provider artifact packages so the
# compiler emits the same digest-pinned binding.
{ pkgs, self }:

let
  inherit (pkgs) lib;
  hostToolBundle =
    if self.lib ? d2bHostToolBundle then self.lib.d2bHostToolBundle else null;
  d2bLib = import ./lib.nix {
    inherit self;
    inherit lib;
    inherit hostToolBundle;
  };
  cloudHypervisorArtifact = d2bLib.mkRuntimeCloudHypervisorArtifact pkgs;
  volumeProviderArtifact = d2bLib.mkVolumeProviderArtifact pkgs;

  # A Provider artifact that packages the Device worker executables the
  # declared rows name. Shape mirrors `mkVolumeProviderArtifact`: a signed
  # manifest whose executable set is computed from the packaged `bin/` files,
  # a Device-exporting catalog entry, and a deterministic publisher key.
  mkDeviceWorkerProviderArtifact =
    { artifactId
    , publisher
    , binaries
    , controllerBinary
    }:
    let
      signer = pkgs.python3.withPackages
        (pythonPackages: [ pythonPackages.cryptography ]);
      manifest = ../../tests/fixtures/provider-acceptance/provider-manifest.json;
      schema = ../../tests/fixtures/provider-acceptance/config-schema.json;
      controller = if hostToolBundle == null then
        "${self.packages.${pkgs.stdenv.hostPlatform.system}.d2b-provider-test-controller}/bin/d2b-provider-test-controller"
      else
        "${hostToolBundle}/bin/d2b-provider-test-controller";
      package = pkgs.runCommand "d2b-${artifactId}" {
        nativeBuildInputs = [ pkgs.coreutils signer ];
      } ''
        mkdir -p "$out/bin"
        ${lib.concatStringsSep "\n" (lib.mapAttrsToList
          (name: path: ''
            cp "${path}" "$out/bin/${name}"
            chmod 0755 "$out/bin/${name}"
          '')
          binaries)}
        cp "${controller}" "$out/bin/${controllerBinary}"
        chmod 0755 "$out/bin/${controllerBinary}"
        ${signer}/bin/python3 - "${manifest}" "$out" \
          "${artifactId}" "${publisher}" "${controllerBinary}" \
          ${lib.escapeShellArg (lib.concatStringsSep " " (lib.attrNames binaries))} <<'PY'
        import hashlib
        import json
        import pathlib
        import sys
        from cryptography.hazmat.primitives import serialization
        from cryptography.hazmat.primitives.asymmetric.ed25519 import (
            Ed25519PrivateKey,
        )

        (
            manifest_path,
            output_path,
            artifact_id,
            publisher,
            controller_binary,
            binary_names,
        ) = sys.argv[1:]
        output = pathlib.Path(output_path)
        manifest = json.loads(pathlib.Path(manifest_path).read_text())
        # The executable set the compiler recomputes covers every regular file
        # in bin/: the controller binary plus the declared worker binaries.
        names = sorted(set(binary_names.split()) | {controller_binary})

        # Device-only manifest: the declared Device worker rows are the only
        # rows this artifact's Provider serves in this fixture.
        resource_types = {"Device"}
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

        manifest["artifactId"] = artifact_id
        manifest["trust"]["publisher"] = publisher
        executable_map = json.dumps(
            {
                name: "sha256:" + hashlib.sha256(
                    (output / "bin" / name).read_bytes()
                ).hexdigest()
                for name in names
            },
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode()
        first = hashlib.sha256(
            b"d2b:v3:provider-executable-set\0" + executable_map
        ).digest()
        executable_digest = "sha256:" + hashlib.sha256(first).hexdigest()
        controller_digest = "sha256:" + hashlib.sha256(
            (output / "bin" / controller_binary).read_bytes()
        ).hexdigest()
        manifest["digests"]["executable"] = executable_digest
        for component in manifest.get("components", []):
            for capability in component.get("targetCapabilities", []):
                capability["artifactDigest"] = controller_digest
        manifest_bytes = json.dumps(
            manifest,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode()
        seed = hashlib.sha256(
            b"d2b-u17-device-worker-provider-signing-key-v1"
            + artifact_id.encode()
        ).digest()
        private_key = Ed25519PrivateKey.from_private_bytes(seed)
        public_key = private_key.public_key().public_bytes(
            serialization.Encoding.PEM,
            serialization.PublicFormat.SubjectPublicKeyInfo,
        )
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
        "d2b-${artifactId}-nar-digest" {
          nativeBuildInputs = [ pkgs.nix ];
        } ''
          printf 'sha256:%s' \
            "$(${pkgs.nix}/bin/nix --extra-experimental-features nix-command \
              hash path --type sha256 --base16 "${package}")" > "$out"
        '';
      baseManifest = builtins.fromJSON (builtins.readFile manifest);
      catalog = {
        providerName = artifactId;
        packageName = "d2b-${artifactId}";
        version = "0.0.0";
        systems = [ pkgs.stdenv.hostPlatform.system ];
        platform = pkgs.stdenv.hostPlatform.system;
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
        supportContact = "d2b-u17-device-worker@localhost";
        publisher = publisher;
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
        configDigest = "sha256:${builtins.hashString
          "sha256" (builtins.readFile schema)}";
      };
    in {
      inherit package catalog;
      type = "provider";
      trustedPublisher = {
        publisherRef = publisher;
        signingKey = builtins.readFile "${package}/publisher-public-key.pem";
      };
    };

  # The GPU artifact's crosvm stand-in: a real ELF (via the shim) so the
  # artifact validates as a Provider executable set, whose behavior is to
  # record its argv and refuse. The fixture never asks a GPU worker to serve.
  crosvmStandIn = self.lib.buildProviderElfShim {
    inherit pkgs;
    name = "crosvm";
    interpreterPkg = pkgs.bash;
    interpreterPath = "bin/bash";
    program = pkgs.writeText "d2b-u17-crosvm-stand-in.sh" ''
      # Fixture stand-in for the GPU Provider's crosvm. It must never run in
      # a passing fixture (the launch path is proved up to the broker's own
      # refusal); if it does run, it records the argv the Process controller
      # composed and refuses, so a fabricated success is impossible.
      set -eu
      log="/run/d2b/u17-device-worker-standin.argv"
      if [ -d /run/d2b ]; then
        printf '%s\n' "crosvm-stand-in:$*" >> "$log" 2>/dev/null || true
      fi
      printf 'd2b-u17: crosvm stand-in invoked with %s\n' "$*" >&2
      exit 79
    '';
  };

  # One artifact serves both Device Providers: a Provider artifact exports its
  # ResourceTypes, and two artifacts both exporting `Device` collide in one
  # Zone (`provider-resourcetype-collision`).
  deviceWorkerArtifact = mkDeviceWorkerProviderArtifact {
    artifactId = "device-worker-acceptance-provider";
    publisher = "d2b-u17-device-worker";
    controllerBinary = "acceptance-controller";
    binaries = {
      swtpm = "${pkgs.swtpm}/bin/swtpm";
      swtpm-ioctl = "${pkgs.swtpm}/bin/swtpm_ioctl";
      crosvm = "${crosvmStandIn}/bin/crosvm";
    };
  };

  cloudHypervisorConfig = {
    controllerExecutionRef = "Host/host-system";
    defaultVcpus = 2;
    defaultMemoryMb = 512;
    defaultMachineType = "microvm";
    watchdog = true;
    adoptionWindowMs = 30000;
    healthCheckIntervalMs = 5000;
    healthCheckTimeoutMs = 1000;
    healthCheckFailureThreshold = 3;
    startupDeadlineMs = 120000;
  };

  artifacts = {
    runtime-cloud-hypervisor = {
      inherit (cloudHypervisorArtifact) package type catalog;
    };
    volume-acceptance-provider = {
      inherit (volumeProviderArtifact) package type catalog;
    };
    device-worker-acceptance-provider = {
      inherit (deviceWorkerArtifact) package type catalog;
    };
  };
in
pkgs.testers.runNixOSTest {
  name = "d2b-device-worker-launch";

  nodes.machine = d2bLib.d2bDaemonNode {
    extra = { ... }: {
      d2b.site.adminUsers = [ "alice" ];
      environment.systemPackages = with pkgs; [
        jq
        procps
        util-linux
        acl
        iproute2
        # The fixture binds its stale video socket from the VM side; python3
        # is the smallest reliable AF_UNIX binder available in the VM.
        python3
      ];
      d2b.artifacts = artifacts;
      d2b.zones.local-root.resources.host-system = {
        type = "Host";
        spec = {
          providerRef = "Provider/system-core";
          defaultDomain = "system";
          allowedDomains = [ "system" ];
          budget = { };
          networkAttachments = [ ];
          deviceAttachments = [ ];
          volumeAttachmentDefaults = [ ];
        };
      };
      d2b.zones.work.parentZone = "local-root";
      # Every Zone the host compiles a bundle for declares the publishers of
      # the artifacts its rows select; `local-root` is a compiled Zone too
      # (the other Cloud Hypervisor fixtures declare the same pair there).
      d2b.zones.local-root.trustedPublishers.d2b-cloud-hypervisor.signingKey =
        cloudHypervisorArtifact.trustedPublisher.signingKey;
      d2b.zones.local-root.trustedPublishers.d2b-volume-acceptance.signingKey =
        volumeProviderArtifact.trustedPublisher.signingKey;
      d2b.zones.local-root.trustedPublishers.d2b-u17-device-worker.signingKey =
        deviceWorkerArtifact.trustedPublisher.signingKey;
      d2b.zones.work.trustedPublishers.d2b-cloud-hypervisor.signingKey =
        cloudHypervisorArtifact.trustedPublisher.signingKey;
      d2b.zones.work.trustedPublishers.d2b-volume-acceptance.signingKey =
        volumeProviderArtifact.trustedPublisher.signingKey;
      d2b.zones.work.trustedPublishers.d2b-u17-device-worker.signingKey =
        deviceWorkerArtifact.trustedPublisher.signingKey;
      d2b.zones.work.resources = {
        alice = {
          type = "User";
          spec = {
            displayName = "Alice";
            groups = [ ];
            osUsername = "alice";
          };
        };
        d2bd = {
          type = "User";
          spec = {
            displayName = "d2bd";
            groups = [ ];
            osUsername = "d2bd";
          };
        };
        device-operator = {
          type = "Role";
          spec.rules = [
            {
              resourceTypes = [
                "Device"
                "Endpoint"
                "EphemeralProcess"
                "Guest"
                "Host"
                "Process"
                "Provider"
                "Volume"
              ];
              verbs = [ "get" "list" ];
              subresources = [ ];
              resourceNames = [ ];
              zones = [ "work" ];
              executionRefs = [ ];
              sessionVerbs = [ "connect" "invoke" ];
            }
            {
              resourceTypes = [ "Device" ];
              verbs = [ "delete" ];
              subresources = [ ];
              resourceNames = [ "tpm0" ];
              zones = [ "work" ];
              executionRefs = [ ];
              sessionVerbs = [ "connect" "invoke" ];
            }
          ];
        };
        device-operator-binding = {
          type = "RoleBinding";
          spec = {
            roleRef = "Role/device-operator";
            subjects = [ "User/alice" ];
            externalPrincipalSelector = null;
            scopeNarrowing = null;
          };
        };
        host-system = {
          type = "Host";
          spec = {
            providerRef = "Provider/system-core";
            defaultDomain = "system";
            allowedDomains = [ "system" ];
            budget = { };
            networkAttachments = [ ];
            deviceAttachments = [ ];
            volumeAttachmentDefaults = [ ];
          };
        };
        # The Device owners. The Guest stays declared input and is never
        # booted: the Device worker rows are bundle-declared `Process` rows of
        # the Process controller, and no guest system artifact is declared, so
        # no VMM is ever launched here. Its name is the VM identity of the
        # Devices it owns (`Device.metadata.ownerRef`).
        acceptance-guest = {
          type = "Guest";
          spec = {
            providerRef = "Provider/volume-virtiofs";
            executionRef = "Host/host-system";
            defaultDomain = "system";
            allowedDomains = [ "system" ];
            budget = { };
            volumeAttachmentDefaults = [ ];
            networkAttachments = [ ];
            deviceAttachments = [ ];
          };
        };
        volume-local = {
          type = "Provider";
          spec = {
            artifactId = "volume-acceptance-provider";
            config = {
              controllerExecutionRef = "Host/host-system";
              sourcePolicies = [
                {
                  id = "daemon-state";
                  class = "local-path";
                  volumeKinds = [ "durable" "state" "cache" ];
                }
                # The TPM state Volume's source policy
                # (`build_tpm_state_volume_spec`, opaque policy id).
                {
                  id = "tpm-state";
                  class = "local-path";
                  volumeKinds = [ "state" ];
                }
              ];
            };
          };
        };
        volume-virtiofs = {
          type = "Provider";
          spec = {
            artifactId = "volume-acceptance-provider";
            config.controllerExecutionRef = "Host/host-system";
          };
        };
        runtime-cloud-hypervisor = {
          type = "Provider";
          spec = {
            artifactId = "runtime-cloud-hypervisor";
            config = cloudHypervisorConfig;
          };
        };
        device-tpm = {
          type = "Provider";
          spec = {
            artifactId = "device-worker-acceptance-provider";
            config.controllerExecutionRef = "Host/host-system";
          };
        };
        device-gpu = {
          type = "Provider";
          spec = {
            artifactId = "device-worker-acceptance-provider";
            config.controllerExecutionRef = "Host/host-system";
          };
        };
        # The Device under test: an emulated TPM claimed by the Guest. The
        # Provider's projection declares `Process/swtpm-tpm0`,
        # `EphemeralProcess/swtpm-flush-tpm0`, `Endpoint/tpm-tpm0` and
        # `Endpoint/tpm-ctrl-tpm0` as this Device's children.
        tpm0 = {
          type = "Device";
          metadata.ownerRef = "Guest/acceptance-guest";
          spec = {
            providerRef = "Provider/device-tpm";
            deviceClass = "emulated";
            arbitration = "exclusive";
            maxConcurrentClaims = 1;
            inventory.selector = { };
          };
        };
        # The GPU/video Devices: a full GPU with its video sidecar
        # (`gpu-worker` + `video-worker` rows) and a render-node-only Device
        # (`gpu-render-node` row, the shape whose render node the broker
        # pre-opens itself). Both are physical DRM Devices by declaration; the
        # VM has no GPU, which is exactly what the fixture measures.
        gpu0 = {
          type = "Device";
          metadata.ownerRef = "Guest/acceptance-guest";
          spec = {
            providerRef = "Provider/device-gpu";
            deviceClass = "physical";
            arbitration = "exclusive";
            maxConcurrentClaims = 1;
            inventory.selector = { busClass = "drm"; label = "u17-gpu0"; };
          };
        };
        gpu1 = {
          type = "Device";
          metadata.ownerRef = "Guest/acceptance-guest";
          spec = {
            providerRef = "Provider/device-gpu";
            deviceClass = "physical";
            arbitration = "exclusive";
            maxConcurrentClaims = 1;
            inventory.selector = { busClass = "drm"; label = "u17-gpu1"; };
          };
        };
      };
    };
  };

  testScript = ''
    ${d2bLib.fixtureDiagnostics}

    import json as _json
    import time as _time
    from typing import Any

    ZONE = "work"
    GUEST = "acceptance-guest"
    DEVICES = ["tpm0", "gpu0", "gpu1"]
    DECLARED_ROWS = {
        # row ref -> (resource type, declared template, owning Device)
        "swtpm-tpm0": ("Process", "swtpm-socket", "Device/tpm0"),
        "swtpm-flush-tpm0": ("EphemeralProcess", "swtpm-init-flush", "Device/tpm0"),
        "gpu-gpu0": ("Process", "gpu-worker", "Device/gpu0"),
        "gpu-gpu1": ("Process", "gpu-worker", "Device/gpu1"),
    }
    # The template binding's owner is the Device *Provider* that signs the
    # template (the declared row's own owner is the Device).
    BINDING_OWNER = {
        "Device/tpm0": "Provider/device-tpm",
        "Device/gpu0": "Provider/device-gpu",
        "Device/gpu1": "Provider/device-gpu",
    }
    UUID_V4 = (
        r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-"
        r"[0-9a-f]{12}$"
    )

    def check(condition, message):
        if not condition:
            raise AssertionError(message)

    def d2b(command, out):
        return (
            "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
            f"d2b --zone {ZONE} --json {command} >{out}"
        )

    def list_json(kind, out):
        return d2b(f"list {kind}", out)

    # The declared flush row (`EphemeralProcess/swtpm-flush-tpm0`) is read by
    # exact ref instead of by a zone-wide `list EphemeralProcess`: the daemon
    # routes every EphemeralProcess List into its exec-session surface
    # (packages/d2bd/src/composition.rs, `process_resource_management_request`
    # -> `dispatch_resource_exec_request`) before the manager sees it, and
    # that surface derives an execution ref from a Guest executionRef or from
    # a resourceRef, neither of which a zone-wide list carries - so the
    # command refuses and never reaches the manager. Get is not intercepted
    # and serves the same row projection; the rows-ingested stage records the
    # refused list once as evidence.
    FLUSH_ROW = "swtpm-flush-tpm0"

    def flush_get(out):
        return d2b(f"get EphemeralProcess/{FLUSH_ROW}", out)

    ROW_FIELDS = (
        "{type: .type, name: .metadata.name, owner: .metadata.ownerRef, "
        "uid: .metadata.uid, gen: .metadata.generation, "
        "obs: .status.observedGeneration, phase: .status.phase, "
        "template: .spec.template, resource: .status.resource}"
    )
    ROW_PROJECTION = f"[.resources[] | {ROW_FIELDS}]"
    # `Get` returns the row itself, so its projection is the same fields
    # without the List envelope.
    FLUSH_PROJECTION = f"[. | {ROW_FIELDS}]"
    row_dumps = [
        (
            f"{kind} rows",
            list_json(kind, f"/run/d2b-u17-{kind.lower()}.json")
            + f" && jq -c '{ROW_PROJECTION}' /run/d2b-u17-{kind.lower()}.json "
            + "|| true",
        )
        for kind in [
            "Device",
            "Process",
            "Endpoint",
            "Volume",
            "Provider",
        ]
    ] + [
        (
            "declared flush row (Get) and zone-wide list EphemeralProcess (exec-routed)",
            flush_get("/run/d2b-u17-ephemeralprocess-get.json")
            + f" && jq -c '{FLUSH_PROJECTION}' "
            "/run/d2b-u17-ephemeralprocess-get.json; "
            "rc=0; "
            + d2b("list EphemeralProcess 2>&1", "/run/d2b-u17-ephemeralprocess.json")
            + " || rc=$?; echo list-exit=$rc; true",
        ),
        (
            "zone resource bundle",
            "jq -c '{resources: [.resources[] | select(.type == \"Process\" or "
            ".type == \"EphemeralProcess\") | {type, name: .metadata.name, "
            "owner: .metadata.ownerRef, template: .spec.template}], "
            "bindings: [.processTemplates[] | {processRef, ownerRef, template, "
            "launchArgs, binaryRef}]}' /etc/d2b/zones/work/resource-bundle.json "
            "|| true",
        ),
        (
            "swtpm storage rows",
            "jq -c '[.paths[] | select(.id | test(\"swtpm\")) | {id, scope, "
            "pathTemplate}]' /etc/d2b/storage.json || true",
        ),
        (
            "site artifact",
            "cat /etc/d2b/site.json 2>/dev/null || echo 'no site.json'",
        ),
        (
            "device worker sockets",
            "find /run/d2b/vms /run/d2b-video -maxdepth 3 "
            "-printf '%M %u:%g %p\\n' 2>/dev/null | sort | head -n 40 || true",
        ),
    ]

    start_all()
    stage("daemon-up")
    diag_unit("daemon-up", "d2bd.service", 180)
    machine.wait_for_unit("d2b-broker.socket", timeout=30)
    machine.wait_for_file("/run/d2b/public.sock", timeout=30)
    machine.succeed("systemctl start d2b-broker.service")

    # 1. Slice 1's compile-time half, on the live host: the Zone bundle carries
    #    exactly one declared row per Device worker template, owned by its
    #    Device, each bound to the Provider's digest-pinned executable with
    #    launch arguments admitted. No hardware and no launch involved.
    stage("bundle-projection")
    bundle_rows: Any = _json.loads(
        machine.succeed("cat /etc/d2b/zones/work/resource-bundle.json")
    )
    declared: Any = {
        (row["type"], row["metadata"]["name"]): row
        for row in bundle_rows["resources"]
        if row["type"] in ("Process", "EphemeralProcess")
    }
    for name, (kind, template, owner) in DECLARED_ROWS.items():
        row = declared.get((kind, name))
        check(row is not None, f"declared row {kind}/{name} missing from the bundle")
        check(
            row["metadata"].get("ownerRef") == owner,
            f"{kind}/{name}: declared owner {row['metadata'].get('ownerRef')!r} "
            f"!= {owner!r}",
        )
        check(
            row["spec"]["template"] == template,
            f"{kind}/{name}: declared template {row['spec']['template']!r} "
            f"!= {template!r}",
        )
        check(
            "command" not in row["spec"] and "argv" not in row["spec"],
            f"{kind}/{name}: the declared row must stay argv-free",
        )
    bindings: Any = {
        binding["processRef"]: binding
        for binding in bundle_rows.get("processTemplates", [])
    }
    print(
        "[d2b] compiled processTemplates: "
        + _json.dumps(
            [
                {
                    "processRef": binding.get("processRef"),
                    "ownerRef": binding.get("ownerRef"),
                    "template": binding.get("template"),
                    "binaryRef": binding.get("binaryRef"),
                    "launchArgs": binding.get("launchArgs", False),
                }
                for binding in bundle_rows.get("processTemplates", [])
            ],
            sort_keys=True,
        )
    )
    for name, (kind, template, owner) in DECLARED_ROWS.items():
        ref = f"{kind}/{name}"
        binding = bindings.get(ref)
        check(binding is not None, f"no Device worker template binding for {ref}")
        check(
            binding["template"] == template,
            f"{ref}: binding template {binding['template']!r} != {template!r}",
        )
        check(
            binding.get("launchArgs") is True,
            f"{ref}: binding must admit launch arguments (saw "
            f"{binding.get('launchArgs')!r})",
        )
        check(
            binding["ownerRef"] == BINDING_OWNER[owner],
            f"{ref}: binding owner {binding['ownerRef']!r} "
            f"!= {BINDING_OWNER[owner]!r}",
        )
        expected_binary = {
            "swtpm-socket": "swtpm",
            "swtpm-init-flush": "swtpm-ioctl",
            "gpu-worker": "crosvm",
            "gpu-render-node": "crosvm",
            "video-worker": "crosvm",
        }[template]
        check(
            binding["binaryRef"] == expected_binary,
            f"{ref}: binding binary {binding['binaryRef']!r} != {expected_binary!r}",
        )
    print(
        "[d2b] declared device worker bindings: "
        + _json.dumps(
            [
                {
                    "processRef": ref,
                    "template": binding["template"],
                    "binaryRef": binding["binaryRef"],
                    "launchArgs": binding.get("launchArgs", False),
                }
                for ref, binding in sorted(bindings.items())
            ],
            sort_keys=True,
        )
    )

    # 2. The rows are ingested into the manager with their Device owner, so the
    #    Process controller is the component that launches them (KTD13). The
    #    three declared Process rows are the whole Process set this stage
    #    asserts (the fourth declared row is the flush EphemeralProcess below),
    #    each present with its Device owner, declared template, and a real v4
    #    uid.
    declared_process_triples = " or ".join(
        f"(.metadata.name == \"{name}\" and .metadata.ownerRef == \"{owner}\" "
        f"and .spec.template == \"{template}\")"
        for name, (kind, template, owner) in DECLARED_ROWS.items()
        if kind == "Process"
    )
    stage("rows-ingested")
    diag_wait(
        "rows-ingested",
        f"{list_json('Process', '/run/d2b-u17-ingest-process.json')} && "
        f"{flush_get('/run/d2b-u17-ingest-flush.json')} && "
        "jq -e --slurpfile flush /run/d2b-u17-ingest-flush.json '"
        "([.resources[] | select(.type == \"Process\" and "
        "(.metadata.name | test(\"^(swtpm-tpm0|gpu-gpu0|gpu-gpu1)$\")))] "
        "| length) == 3 and "
        "([.resources[] | select(.type == \"Process\" and "
        "(.metadata.name | test(\"^(swtpm-tpm0|gpu-gpu0|gpu-gpu1)$\")) "
        "and (.metadata.uid | test(\"" + UUID_V4 + "\")))] | length) == 3 and "
        "([.resources[] | select(.type == \"Process\" and "
        "(" + declared_process_triples + "))] | length) == 3 and "
        "([$flush[0] | select(.type == \"EphemeralProcess\" and "
        ".metadata.name == \"swtpm-flush-tpm0\" and "
        ".metadata.ownerRef == \"Device/tpm0\" and "
        ".spec.template == \"swtpm-init-flush\" and "
        "(.metadata.uid | test(\"" + UUID_V4 + "\")))] | length) == 1' "
        "/run/d2b-u17-ingest-process.json",
        timeout=180,
        rows=row_dumps,
        explain=[("d2bd.service", "device-worker")],
    )
    for name, (kind, template, owner) in DECLARED_ROWS.items():
        if kind == "Process":
            source = "/run/d2b-u17-ingest-process.json"
            row_source = (
                f".resources[] | select(.type == \"{kind}\" and "
                f".metadata.name == \"{name}\")"
            )
        else:
            source = "/run/d2b-u17-ingest-flush.json"
            row_source = "."
        output = machine.succeed(
            f"jq -c '[{row_source} | {{owner: .metadata.ownerRef, "
            "template: .spec.template, phase: .status.phase}]' " + source
        )
        rows = _json.loads(output)
        check(len(rows) == 1, f"{kind}/{name}: expected one ingested row, got {output}")
        check(
            rows[0]["owner"] == owner and rows[0]["template"] == template,
            f"{kind}/{name}: ingested shape {output}",
        )

    # Evidence for the read path above (not an assertion): a zone-wide
    # `list EphemeralProcess` is exec-routed and never reaches the manager, so
    # record its exit status and stderr once per run. The declared row itself
    # is read by exact ref through `flush_get` instead.
    list_probe = machine.execute(
        d2b("list EphemeralProcess 2>&1", "/run/d2b-u17-ephemeralprocess-probe.json")
    )
    print(
        f"[d2b] zone-wide list EphemeralProcess probe: exit {list_probe[0]}; "
        + list_probe[1].strip().replace("\n", " | ")
    )

    # 3. Launch-outcome evidence (diagnostic, not an assertion): every
    #    declared row reaches either Ready or a terminal classification within
    #    the bounded window, and the log carries the classification the row
    #    and the daemon publish. A row still Pending here is a launch that
    #    never resolved; the stages below assert the target behavior.
    stage("worker-launch-outcome")
    outcome_deadline = _time.monotonic() + 150
    while _time.monotonic() < outcome_deadline:
        machine.succeed(
            list_json("Process", "/run/d2b-u17-outcome-process.json") + "; echo OK"
        )
        machine.succeed(
            flush_get("/run/d2b-u17-outcome-flush.json") + "; echo OK"
        )
        rows_now = _json.loads(
            machine.succeed(
                "jq -c '[.resources[] | select(.metadata.name | "
                "test(\"^(swtpm-tpm0|gpu-gpu0|gpu-gpu1)$\")) | "
                "{name: .metadata.name, phase: .status.phase, "
                "resource: .status.resource}]' /run/d2b-u17-outcome-process.json"
                " && echo '---' && jq -c '[select(.metadata.name == "
                "\"swtpm-flush-tpm0\") | {name: .metadata.name, "
                "phase: .status.phase, resource: .status.resource}]' "
                "/run/d2b-u17-outcome-flush.json"
            ).split("---")[0]
        )
        flat_now = rows_now
        settled = [
            row for row in flat_now
            if row["phase"] in ("Ready", "Failed", "Quarantined")
        ]
        if len(settled) == len(flat_now) and flat_now:
            break
        _time.sleep(0.5)
    for row in _json.loads(
        machine.succeed(
            "jq -c '[.resources[] | select(.metadata.name | "
            "test(\"^(swtpm-tpm0|gpu-gpu0|gpu-gpu1)$\")) | "
            "{name: .metadata.name, owner: .metadata.ownerRef, "
            "phase: .status.phase, resource: .status.resource}]' "
            "/run/d2b-u17-outcome-process.json"
        )
    ):
        print("[d2b] declared row outcome: " + _json.dumps(row, sort_keys=True))
    print(
        "[d2b] flush row outcome: "
        + machine.succeed(
            "jq -c '[select(.metadata.name == \"swtpm-flush-tpm0\") | "
            "{phase: .status.phase, resource: .status.resource}]' "
            "/run/d2b-u17-outcome-flush.json"
        )
    )
    print(
        "[d2b] launch refusal lines:\n"
        + machine.succeed(
            "journalctl -u d2bd.service --no-pager -o cat -b -n 4000 "
            "| grep -E 'device-worker|process-resolution-refused|"
            "provider-ticket|swtpm|w1-gpu' | tail -n 40 || true"
        )
    )

    # 4. TPM end to end. The declared `Process/swtpm-tpm0` row is launched by
    #    the Process controller with the parameters the Device row, the
    #    declared template, and the daemon runtime paths supply: it reaches
    #    Ready, the real swtpm process is alive with that argv, and its
    #    sockets exist.
    stage("tpm-worker")
    diag_wait(
        "tpm-worker-ready",
        f"{list_json('Process', '/run/d2b-u17-tpm-process.json')} && "
        "jq -e '([.resources[] | select(.type == \"Process\" and "
        ".metadata.name == \"swtpm-tpm0\") | select("
        ".status.phase == \"Ready\" and "
        ".status.observedGeneration == .metadata.generation and "
        ".metadata.ownerRef == \"Device/tpm0\" and "
        ".spec.template == \"swtpm-socket\")] | length) == 1' "
        "/run/d2b-u17-tpm-process.json",
        timeout=180,
        rows=row_dumps,
        explain=[("d2bd.service", "swtpm"), ("d2b-broker.service", "w1-swtpm")],
    )
    diag_wait(
        "tpm-worker-process",
        "test \"$(ps -eo args= | awk '/[s]wtpm socket/ {c++} END {print c+0}')\" -ge 1",
        timeout=60,
        rows=row_dumps,
        explain=[("d2bd.service", "swtpm")],
    )
    argv = machine.succeed(
        "for pid in $(pgrep -f '[s]wtpm socket'); do tr '\\0' ' ' < /proc/$pid/cmdline; "
        "echo; done"
    ).strip()
    print("[d2b] live swtpm argv: " + argv)
    check(
        "--tpm2" in argv and "--ctrl" in argv and "--server" in argv
        and "--tpmstate" in argv,
        f"the live swtpm argv must be the composed swtpm shape: {argv!r}",
    )
    check(
        f"path=/run/d2b/vms/{GUEST}/tpm.sock" in argv,
        f"the live swtpm argv must carry the per-VM server socket: {argv!r}",
    )
    check(
        "device-" in argv and "tpm-state/ctrl.sock" in argv,
        f"the live swtpm argv must carry the controller-created state dir: {argv!r}",
    )
    state_dir = machine.succeed(
        "for pid in $(pgrep -f '[s]wtpm socket'); do "
        "tr '\\0' '\\n' < /proc/$pid/cmdline | sed -n '/--pid/{n;p}'; done "
        "| head -n1 | sed 's|^file=||' | xargs -r dirname"
    ).strip()
    check(
        state_dir.startswith("/var/lib/d2b/") and state_dir.endswith("tpm-state"),
        f"the swtpm state dir must be the controller-created Volume root: {state_dir!r}",
    )
    diag_wait(
        "tpm-sockets",
        f"test -S /run/d2b/vms/{GUEST}/tpm.sock && test -S {state_dir}/ctrl.sock",
        timeout=60,
        rows=row_dumps,
        explain=[("d2bd.service", "swtpm")],
    )
    print(
        "[d2b] swtpm state dir: "
        + state_dir
        + "; sockets: "
        + machine.succeed(
            f"stat -c '%F %a %U:%G %n' /run/d2b/vms/{GUEST}/tpm.sock {state_dir}/ctrl.sock"
        ).replace("\n", " | ")
    )

    # 4. The one-shot flush publishes its outcome as the row's status
    #    projection, which is what the TPM port's flush gate reads.
    stage("tpm-flush")
    diag_wait(
        "tpm-flush-outcome",
        f"{flush_get('/run/d2b-u17-flush-process.json')} && "
        "jq -e '([select(.type == \"EphemeralProcess\" and "
        ".metadata.name == \"swtpm-flush-tpm0\" and "
        ".metadata.ownerRef == \"Device/tpm0\" and "
        ".spec.template == \"swtpm-init-flush\" and "
        ".status.resource.ephemeral.state == \"succeeded\" and "
        ".status.resource.ephemeral.code == \"process-exited\")] | length) == 1' "
        "/run/d2b-u17-flush-process.json",
        timeout=180,
        rows=row_dumps,
        explain=[("d2bd.service", "swtpm-flush"), ("d2bd.service", "ephemeral")],
    )
    print(
        "[d2b] flush outcome projection: "
        + machine.succeed(
            "jq -c '[.status.resource]' /run/d2b-u17-flush-process.json"
        )
    )

    # 5. GPU: the launch path, honestly. The VM has no GPU, so the fixture
    #    states what must happen instead of faking a device: the declared rows
    #    resolve, their launch is attempted through the Process controller and
    #    the broker, and every GPU row ends on a named refusal - never Ready,
    #    never a bare launch. (The GPU Device templates that need Provider
    #    settings - `video-worker`, `gpu-render-node` - would need the
    #    Provider's signed settings schema registered in the zone; this
    #    fixture declares the plain `gpu-worker` template on both Devices.)
    stage("gpu-launch")
    # The GPU rows must end on a named refusal, never Ready and never a fake
    # device. The row status (and the daemon journal) name the stage.
    observed: Any = None
    deadline = _time.monotonic() + 180
    while _time.monotonic() < deadline:
        machine.succeed(
            list_json("Process", "/run/d2b-u17-gpu-process.json") + "; echo OK"
        )
        rows = _json.loads(
            machine.succeed(
                "jq -c '{gpu0: [.resources[] | select(.metadata.name == \"gpu-gpu0\") "
                "| {phase: .status.phase, resource: .status.resource}], "
                "gpu1: [.resources[] | select(.metadata.name == \"gpu-gpu1\") "
                "| {phase: .status.phase, resource: .status.resource}]}' "
                "/run/d2b-u17-gpu-process.json"
            )
        )
        flat = rows["gpu0"] + rows["gpu1"]
        terminal = [
            row for row in flat if row["phase"] in ("Failed", "Quarantined")
        ]
        if len(terminal) == 2 or any(row["phase"] == "Ready" for row in flat):
            observed = rows
            break
        _time.sleep(0.5)
    check(
        observed is not None,
        "every GPU/video row must reach a terminal refusal without a GPU",
    )
    flat = observed["gpu0"] + observed["gpu1"]
    check(
        len(flat) == 2 and all(row["phase"] == "Failed" for row in flat),
        "every GPU/video row must end Failed without a GPU: "
        + _json.dumps(observed, sort_keys=True),
    )
    for row in flat:
        # The refusal is the closed classification the plan documents: the
        # restart ceiling makes a persistently refused launch terminal as
        # `process-start-budget-exhausted` at the launch stage
        # (`reconcile/launch`), never Ready and never a bare failure.
        failure = (row["resource"] or {}).get("driverFailure") or {}
        check(
            failure.get("code") == "process-start-budget-exhausted"
            and failure.get("stage") == "reconcile/launch",
            "the GPU/video refusal must be the budget-exhausted launch refusal "
            "(process-start-budget-exhausted at reconcile/launch): "
            + _json.dumps(row, sort_keys=True),
        )
    print(
        "[d2b] GPU/video terminal refusals: " + _json.dumps(observed, sort_keys=True)
    )
    codes = machine.succeed(
        "journalctl -u d2bd.service --no-pager -o cat -b -n 4000 "
        "| grep -E 'device-worker|process-resolution-refused|gpu-runner-shape|"
        "render|w1-gpu|video' | tail -n 40 || true"
    )
    print("[d2b] GPU/video refusal evidence:\n" + codes)

    # 6. Teardown: deleting the Device retires its declared rows through the
    #    Process controller - the process stops and the rows go away, children
    #    first, with nothing of the Device left behind.
    stage("tpm-teardown")
    machine.succeed(f"{list_json('Device', '/run/d2b-u17-device-pre-delete.json')}")
    revision = machine.succeed(
        "jq -er '.resources[] | select(.type == \"Device\" and "
        ".metadata.name == \"tpm0\") | .metadata.revision' "
        "/run/d2b-u17-device-pre-delete.json"
    ).strip()
    machine.succeed(
        d2b("delete Device/tpm0 --revision " + revision, "/run/d2b-u17-device-delete.json")
    )
    diag_wait(
        "tpm-teardown",
        f"{list_json('Process', '/run/d2b-u17-teardown-process.json')} && "
        "jq -e 'all(.resources[]; "
        "(.type == \"Process\" and .metadata.name == \"swtpm-tpm0\") | not)' "
        "/run/d2b-u17-teardown-process.json",
        timeout=180,
        rows=row_dumps,
        explain=[("d2bd.service", "swtpm"), ("d2bd.service", "delete")],
    )
    flush_gone = machine.execute(flush_get("/run/d2b-u17-teardown-flush.json"))
    check(
        flush_gone[0] != 0,
        "the Device delete must retire EphemeralProcess/swtpm-flush-tpm0 "
        f"(Get must refuse, saw exit {flush_gone[0]})",
    )
    print(
        "[d2b] flush row after teardown: " + flush_gone[1].strip().replace("\n", " | ")
    )
    check(
        machine.execute("pgrep -f '[s]wtpm socket' >/dev/null")[0] != 0,
        "no swtpm worker may outlive the Device that declared it",
    )
    print("[d2b] Device/tpm0 teardown retired its declared rows and the worker")

    stage("done")
    print("[d2b] U17 device-worker launch path holds on the live host")
  '';
}
