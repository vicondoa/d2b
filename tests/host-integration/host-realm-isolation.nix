# Type-G runNixOSTest: host remains isolated from Gateway Guest relay credentials.
{ pkgs, self }:

let
  inherit (pkgs) lib;
  d2bLib = import ./lib.nix {
    inherit self;
    inherit (pkgs) lib;
  };
  acceptancePublisherKey = ''
    -----BEGIN PUBLIC KEY-----
    MCowBQYDK2VwAyEAu3/qwmKeWeFP7U5Z71uQOw/Zm5lBk4ZDbPVA2O7QlHg=
    -----END PUBLIC KEY-----
  '';
  gatewayCanary = "d2b-u5-gateway-canary-7f4e9c2a";
  gatewayStateDir = "/var/lib/d2b/zones/work/guests/gateway";
  gatewayObservationDir = "${gatewayStateDir}/canary-observation";
  gatewayCredentialDir = "/var/lib/d2b/guest-state/gateway-credentials";
  gatewaySealKeyB64 = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
  gatewaySealKey = pkgs.writeText
    "d2b-u5-gateway-seal-key.b64"
    "${gatewaySealKeyB64}\n";
  gatewayCredentialPython = pkgs.python3.withPackages
    (pythonPackages: [ pythonPackages.cryptography ]);
  gatewayCanaryDigest = builtins.hashString "sha256" gatewayCanary;
  gatewayCredential = pkgs.runCommand "d2b-u5-gateway-sealed-credential" {
    nativeBuildInputs = [ gatewayCredentialPython ];
  } ''
    ${gatewayCredentialPython}/bin/python3 - <<'PY' > "$out"
    import base64
    import json
    from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305

    key = bytes(range(32))
    nonce = bytes(range(12))
    material = {
        "relayListen": {
            "keyName": "d2b-u5-listen",
            "key": "${gatewayCanary}",
        },
        "relaySend": {
            "keyName": "d2b-u5-send",
            "key": "${gatewayCanary}",
        },
    }
    plaintext = json.dumps(material, separators=(",", ":")).encode()
    aad = (
        b"d2b-gateway-credential-v1"
        + (1).to_bytes(8, "big")
        + b"\x00"
        + (0).to_bytes(8, "big")
    )
    ciphertext = ChaCha20Poly1305(key).encrypt(nonce, plaintext, aad)
    envelope = {
        "schemaVersion": 1,
        "generation": 1,
        "notAfter": None,
        "nonce": base64.b64encode(nonce).decode(),
        "ciphertext": base64.b64encode(ciphertext).decode(),
    }
    print(json.dumps(envelope, separators=(",", ":")))
    PY
  '';
  gatewayGuest = self.lib.evalGuest {
    system = pkgs.system;
    name = "gateway";
    zone = "work";
    stateDir = gatewayStateDir;
    modules = [
      ({ lib, pkgs, ... }: {
        environment.etc."d2b/gateway.json".text = builtins.toJSON {
          credentialPath = "${gatewayCredentialDir}/relay.sealed.json";
          sealKeyPath = "${gatewayCredentialDir}/seal.key";
          observationPath = "/run/d2b-gateway-observation/opened";
          relay = {
            namespace = "relns-d2b-prod";
            entity = "hc-d2b-work";
          };
        };
        microvm.shares = lib.mkAfter [
          {
            source = gatewayObservationDir;
            mountPoint = "/run/d2b-gateway-observation";
            tag = "d2b-canary";
            proto = "virtiofs";
            readOnly = false;
          }
        ];
        system.activationScripts.d2bGatewayCredential = {
          deps = [ "users" ];
          text = ''
            install -d -o d2bd -g d2bd -m 0700 ${gatewayCredentialDir}
            ${pkgs.coreutils}/bin/base64 -d ${gatewaySealKey} \
              > ${gatewayCredentialDir}/seal.key
            ${pkgs.coreutils}/bin/chown d2bd:d2bd \
              ${gatewayCredentialDir}/seal.key
            ${pkgs.coreutils}/bin/chmod 0600 \
              ${gatewayCredentialDir}/seal.key
            ${pkgs.coreutils}/bin/install -o d2bd -g d2bd -m 0600 ${gatewayCredential} \
              ${gatewayCredentialDir}/relay.sealed.json
          '';
        };
      })
    ];
  };
  gatewaySystem = gatewayGuest.config.system.build.toplevel;
  providerPackage = pkgs.runCommand "d2b-zone-provider" {
    nativeBuildInputs = [ pkgs.coreutils ];
  } ''
    install -Dm644 ${../../tests/fixtures/provider-acceptance/provider-manifest.json} \
      "$out/share/d2b/provider/provider-manifest.json"
    install -Dm644 ${../../tests/fixtures/provider-acceptance/config-schema.json} \
      "$out/share/d2b/provider/config-schema.json"
    install -Dm755 ${pkgs.coreutils}/bin/true "$out/bin/acceptance-controller"
    base64 -d ${../../tests/fixtures/provider-acceptance/provider-manifest.sig.b64} \
      >"$out/share/d2b/provider/provider-manifest.json.sig"
  '';
  providerCatalog = {
    providerName = "acceptance-provider";
    packageName = "d2b-acceptance-provider";
    version = "0.0.0";
    systems = [ "x86_64-linux" ];
    platform = "x86_64-linux";
    apiCompatibility = "d2b.zone.v3";
    serviceCompatibility = "d2bd.resource";
    signature = "default";
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
    publisher = "d2b-acceptance";
    packageDigest = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    executableDigest = "sha256:f84125779653dba770042fd2af2bd01299b05ae892c039c497e6b5ce45029d9c";
    manifestDigest = "sha256:3c772c723cc2d508502132e10c325a2194c7683025d0c1e8ea9e125d163a10c3";
    componentDigest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    descriptorDigest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    configDigest = "sha256:ccb5a9d66e068ea8f4e205788589675a48e9e3754a840d8ac10120d14238e914";
  };
  providerArtifact = {
    package = providerPackage;
    type = "provider";
    catalog = providerCatalog;
  };
  acceptanceArtifactCatalogDigest =
    "sha256:${lib.concatStringsSep "" (lib.replicate 64 "a")}";
  acceptanceArtifactCatalog = pkgs.writeText "d2b-zone-artifact-catalog.json"
    (builtins.toJSON {
      schemaVersion = 3;
      catalogDigest = acceptanceArtifactCatalogDigest;
      entries = [
        {
          artifactId = "acceptance-provider";
          type = "provider";
          storePath = "${providerPackage}";
          packageDigest = providerCatalog.packageDigest;
          closureDigest = acceptanceArtifactCatalogDigest;
          closureSize = 0;
        }
      ];
    });
in
pkgs.testers.runNixOSTest {
  name = "d2b-host-zone-gateway-isolation";

  nodes.machine = d2bLib.d2bDaemonNode {
    extra = { ... }: {
      environment.systemPackages = [
        pkgs.iproute2
        pkgs.jq
      ];

      d2b.site.usePrebuiltHostTools = false;
      d2b.gateways = lib.mkForce { };
      system.activationScripts.d2bGatewayCanaryObservation = {
        deps = [ "users" ];
        text = ''
          install -d -m 0700 -o d2bd -g d2bd \
            ${gatewayObservationDir}
          ${pkgs.coreutils}/bin/rm -f \
            ${gatewayObservationDir}/opened
        '';
      };
      d2b.artifacts = {
        gateway-system = {
          package = gatewaySystem;
          type = "nixos-system";
        };
        net-vm-base = {
          package = pkgs.writeText "d2b-zone-net-vm" "net-vm";
          type = "nixos-system";
        };
        acceptance-provider = {
          inherit (providerArtifact) package type catalog;
        };
      };
      d2b.providerCatalog = {
        acceptance-provider = {
          artifactId = "acceptance-provider";
        };
      };
      d2b._resourceCompiler.providerProjectionRuntimeCloudHypervisor.resourcesByZone.work.system-minijail = {
        type = "Provider";
        spec = {
          artifactId = "acceptance-provider";
          config = { };
        };
      };
      d2b._artifactCatalogV3 = lib.mkForce {
        catalogDigest = acceptanceArtifactCatalogDigest;
        path = acceptanceArtifactCatalog;
      };
      d2b._bundle.extraArtifacts.artifactCatalog = lib.mkForce {
        data = {
          schemaVersion = 3;
          catalogDigest = acceptanceArtifactCatalogDigest;
          entries = [ ];
        };
        jsonText = builtins.readFile acceptanceArtifactCatalog;
        path = lib.mkForce acceptanceArtifactCatalog;
        installFileName = "artifact-catalog.json";
        classification = "contractPrivateNonSecret";
        sensitivity = "nonSecret";
      };
      d2b.zones.local-root.trustedPublishers.d2b-acceptance.signingKey =
        acceptancePublisherKey;
      d2b.zones.work.trustedPublishers.d2b-acceptance.signingKey =
        acceptancePublisherKey;
      d2b.guestSystems.work.gateway = gatewayGuest;
      d2b.zones.local-root.resources.host = {
        type = "Host";
        spec.providerRef = "Provider/system-core";
      };
      d2b.zones.work = {
        parentZone = "local-root";
        resources = {
          host = {
            type = "Host";
            spec.providerRef = "Provider/system-core";
          };
          gateway = {
            type = "Guest";
            spec = {
              providerRef = "Provider/runtime-cloud-hypervisor";
              systemArtifactId = "gateway-system";
              networkAttachments = [
                {
                  default = true;
                  networkRef = "Network/relay-egress";
                }
              ];
            };
          };
          network-local = {
            type = "Provider";
            spec = {
              artifactId = "acceptance-provider";
              config.controllerExecutionRef = "Host/host";
            };
          };
          relay-egress = {
            type = "Network";
            spec = {
              lanCidr = "10.70.0.0/24";
              providerRef = "Provider/network-local";
              netVmSystemArtifactId = "net-vm-base";
              uplinkCidr = "192.0.2.4/30";
            };
          };
          runtime-cloud-hypervisor = {
            type = "Provider";
            spec = {
              artifactId = "acceptance-provider";
              config.controllerExecutionRef = "Host/host";
            };
          };
          transport-azure-relay = {
            type = "Provider";
            spec = {
              artifactId = "acceptance-provider";
              config = {
                executionRef = "Guest/gateway";
                networkRef = "Network/relay-egress";
              };
            };
          };
          credential-managed-identity = {
            type = "Provider";
            spec = {
              artifactId = "acceptance-provider";
              config = {
                credentialDomains = [ "system" ];
                supportedOperations = [ "acquire-token" ];
              };
            };
          };
          relay-listen = {
            type = "Credential";
            spec = {
              providerRef = "Provider/credential-managed-identity";
              audience = "azure-relay-listen";
              allowedOperations = [ "acquire-token" ];
              consumerRef = "Provider/transport-azure-relay";
              expiry.hardDeadlineMs = 0;
              revocation = {
                onOwnerDelete = "immediate";
                onProviderGeneration = "immediate";
              };
              rotation = {
                maxLeaseLifetimeMs = 0;
                policy = "on-expiry";
                proactiveWindowMs = null;
              };
              scope.executionRef = "Guest/gateway";
            };
          };
          relay-send = {
            type = "Credential";
            spec = {
              providerRef = "Provider/credential-managed-identity";
              audience = "azure-relay-send";
              allowedOperations = [ "acquire-token" ];
              consumerRef = "Provider/transport-azure-relay";
              expiry.hardDeadlineMs = 0;
              revocation = {
                onOwnerDelete = "immediate";
                onProviderGeneration = "immediate";
              };
              rotation = {
                maxLeaseLifetimeMs = 0;
                policy = "on-expiry";
                proactiveWindowMs = null;
              };
              scope.executionRef = "Guest/gateway";
            };
          };
          uplink = {
            type = "ZoneLink";
            spec = {
              childZoneName = "work";
              disabled = false;
              limits = {
                maxActiveStreams = 32;
                maxPendingIntents = 256;
                reconnectMaxAttempts = 10;
                reconnectWindowSecs = 300;
              };
              transportCredentials = [
                "Credential/relay-listen"
                "Credential/relay-send"
              ];
              transportProviderRef = "Provider/transport-azure-relay";
              transportSettings = {
                relayEntityId = "hc-d2b-work";
                relayNamespaceId = "relns-d2b-prod";
              };
            };
          };
        };
      };
    };
  };

  testScript = ''
    start_all()
    machine.wait_for_unit("d2bd.service")
    machine.wait_for_unit("d2b-broker.socket")
    machine.wait_for_file("/run/d2b/public.sock")

    # The committed Process/cloud-hypervisor-gateway row is desired-running;
    # d2bd's startup process reconciliation is the production Guest launcher.
    canary = ${builtins.toJSON gatewayCanary}
    gateway_vsock = "/var/lib/d2b/zones/work/guests/gateway/vsock.sock"
    machine.wait_for_file(gateway_vsock)
    machine.succeed(f"test -S {gateway_vsock}")
    observation_path = (
      "/var/lib/d2b/zones/work/guests/gateway/"
      "canary-observation/opened"
    )
    machine.wait_for_file(observation_path)
    observation = machine.succeed(f"cat {observation_path}").strip()
    assert observation == (
      "schemaVersion=1\n"
      "generation=1\n"
      "digest=sha256:${gatewayCanaryDigest}\n"
    )

    policy = "/etc/d2b/host-realm-relay-egress-policy.json"
    machine.succeed(f"test -r {policy}")
    machine.succeed(
      f"jq -e '.mode == \"host-realm-relay-deny\" "
      f"and (.gatewayInterfaces == []) "
      f"and (.diagnostics.redacted == true) "
      f"and (.diagnostics.rateLimited == true)' {policy}"
    )
    policy_forbidden = [
      "relns-example.servicebus.windows.net",
      "hc-d2b-work",
      "Credential/relay-listen",
      "Credential/relay-send",
      "/var/lib/d2b/gateways/work/credential.sealed.json",
      "/var/lib/d2b/gateways/work/seal.key",
      "SharedAccessKey",
    ]
    for token in policy_forbidden:
      machine.fail(f"grep -F {repr(token)} {policy}")

    runtime_forbidden = policy_forbidden + ["D2B_RELAY_", canary]

    machine.fail("test -e /etc/d2b/gateway.json")
    machine.fail("systemd-tmpfiles --cat-config | grep -F '/var/lib/d2b/gateways/work'")
    machine.succeed("test -r /etc/d2b/zones/work/resource-bundle.json")
    for host_path in [
      "/etc/d2b/zones",
      "/etc/d2b/bundle.json",
      "/etc/d2b/allocator.json",
      "/var/lib/d2b",
      "/run/d2b",
    ]:
      machine.succeed(
        f"! grep -R -F -- {canary!r} {host_path} 2>/dev/null"
      )
    machine.fail(
      "grep -R -F 'SharedAccessKey' /etc/d2b/zones /var/lib/d2b 2>/dev/null"
    )
    machine.succeed(
      f"! journalctl --no-pager -b 2>/dev/null | grep -F -- {canary!r}"
    )
    machine.succeed(
      f"! grep -R -F -- {canary!r} /var/log /var/lib/d2b/audit "
      "2>/dev/null"
    )
    machine.succeed(
      f"! (coredumpctl --no-pager 2>/dev/null || true) "
      f"| grep -F -- {canary!r}"
    )

    pids = machine.succeed("pgrep -x d2bd").strip().split()
    assert pids, "d2bd pid missing"
    machine.succeed("systemctl start d2b-broker.service")
    broker_pid = machine.succeed(
      "for i in $(seq 1 50); do "
      "pid=$(systemctl show -p MainPID --value d2b-broker.service); "
      "if [ -n \"$pid\" ] && [ \"$pid\" != 0 ]; then echo \"$pid\"; exit 0; fi; "
      "sleep 0.2; done; exit 1"
    ).strip()
    pids.append(broker_pid)

    for pid in pids:
      env = machine.succeed(f"tr '\\0' '\\n' < /proc/{pid}/environ || true")
      cmd = machine.succeed(f"tr '\\0' ' ' < /proc/{pid}/cmdline || true")
      fds = machine.succeed(f"ls -l /proc/{pid}/fd || true")
      for token in runtime_forbidden:
        assert token not in env, f"forbidden token leaked in environ for pid {pid}"
        assert token not in cmd, f"forbidden token leaked in cmdline for pid {pid}"
        assert token not in fds, f"forbidden token leaked in fd table for pid {pid}"

    machine.succeed(
      f"! journalctl --no-pager -b 2>/dev/null | grep -F -- {canary!r}"
    )
    machine.succeed(
      f"! grep -R -F -- {canary!r} /etc/d2b /var/lib/d2b /run/d2b "
      "2>/dev/null"
    )
    machine.succeed(
      f"! (coredumpctl --no-pager 2>/dev/null || true) "
      f"| grep -F -- {canary!r}"
    )

    sockets = machine.succeed("ss -Htanp || true")
    assert "servicebus.windows.net" not in sockets
    assert "d2b-provider-relay" not in sockets
    assert "d2b-gateway-relay" not in sockets
  '';
}
