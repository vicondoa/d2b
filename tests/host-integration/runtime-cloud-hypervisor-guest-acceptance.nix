# Type-G runNixOSTest: Guest process adoption across a daemon restart.
#
# This is the lowest public host selector that can exercise the installed
# Cloud Hypervisor runner and pidfd recovery state. It is intentionally not a
# substitute for the native controller acceptance: it proves the production
# daemon restart/process-adoption boundary when the VM prerequisites are
# available.
{ pkgs, self }:

let
  inherit (pkgs) lib;
  d2bLib = import ./lib.nix {
    inherit self;
    inherit lib;
  };
  acceptancePublisherKey = ''
    -----BEGIN PUBLIC KEY-----
    MCowBQYDK2VwAyEA6kpsY+KcUgq+9VB7Ey7F+ZVHdq6+vnuSQh7qaRRG0iw=
    -----END PUBLIC KEY-----
  '';
  providerPackage = pkgs.runCommand "d2b-acceptance-provider" {
    nativeBuildInputs = [ pkgs.coreutils ];
  } ''
    install -Dm644 ${../../tests/fixtures/provider-acceptance/provider-manifest.json} \
      "$out/share/d2b/provider/provider-manifest.json"
    install -Dm644 ${../../tests/fixtures/provider-acceptance/config-schema.json} \
      "$out/share/d2b/provider/config-schema.json"
    install -d -m755 "$out/share/d2b/provider"
    install -Dm755 ${pkgs.coreutils}/bin/true \
      "$out/bin/acceptance-controller"
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
    manifestDigest = "sha256:5f8d852ba3ecd89883afdcf2330f3f752eb1d68a572698035177bcd4b8595e6c";
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
  acceptanceArtifactCatalog = pkgs.writeText "d2b-acceptance-artifact-catalog.json"
    (builtins.toJSON {
      schemaVersion = 3;
      catalogDigest = acceptanceArtifactCatalogDigest;
      entries = map
        (artifactId: {
          inherit artifactId;
          type = "provider";
          storePath = "${providerPackage}";
          packageDigest = providerCatalog.packageDigest;
          closureDigest = acceptanceArtifactCatalogDigest;
          closureSize = 0;
        })
        [ "acceptance-provider" ];
    });
  artifacts = lib.listToAttrs (map
    (artifactId: lib.nameValuePair artifactId providerArtifact)
    [ "acceptance-provider" ]);
in
pkgs.testers.runNixOSTest {
  name = "d2b-runtime-cloud-hypervisor-guest-acceptance";

  nodes.machine = d2bLib.d2bDaemonNode {
    writableStore = true;
    extra = { config, pkgs, ... }: {
      systemd.services.d2bd.serviceConfig.ExecStartPre = lib.mkAfter [
        "+${pkgs.writeShellScript "d2b-runtime-cgroup-prep" ''
          relative=$(sed -n 's/^0:://p' /proc/self/cgroup)
          path="/sys/fs/cgroup''${relative}/cgroup.kill"
          if [ -e "$path" ]; then
            chown d2bd:d2bd "$path" 2>/dev/null || true
            chmod u+w "$path" 2>/dev/null || true
          fi
        ''}"
      ];
      d2b.site.adminUsers = [ "alice" ];
      d2b.vms.corp-vm.env = lib.mkForce null;
      environment.variables.D2B_MANIFEST_PATH = config.d2b._manifestJsonPath;
      environment.systemPackages = with pkgs; [ jq iputils ];
      d2b.artifacts = artifacts;
      d2b._artifactCatalogV3 = lib.mkForce {
        catalogDigest = acceptanceArtifactCatalogDigest;
        path = acceptanceArtifactCatalog;
      };
      d2b._bundle.extraArtifacts.artifactCatalog = lib.mkForce {
        data = { schemaVersion = 3; catalogDigest = acceptanceArtifactCatalogDigest; entries = [ ]; };
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
      d2b.zones.work = {
        parentZone = "local-root";
        resources = {
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
          volume-local = {
            type = "Provider";
            spec = {
              artifactId = "acceptance-provider";
              config = { };
            };
          };
          volume-virtiofs = {
            type = "Provider";
            spec = {
              artifactId = "acceptance-provider";
              config = { };
            };
          };
          corp-vm = {
            type = "Guest";
            spec = {
              providerRef = "Provider/system-core";
              defaultDomain = "system";
              allowedDomains = [ "system" ];
              budget = { };
              volumeAttachmentDefaults = [ ];
              networkAttachments = [ ];
              deviceAttachments = [ ];
            };
          };
          display-wayland = {
            type = "Provider";
            spec = {
              artifactId = "acceptance-provider";
              config = {
                principalPoolSize = 4;
                runtimeVolumePolicyId = "display-wayland.wlproxy-runtime.v1";
              };
            };
          };
          display-wayland-policy = {
            type = "display-wayland.d2bus.org.WaylandPolicy";
            metadata.ownerRef = "Provider/display-wayland";
            spec = {
              allowGlobals = [ ];
              denyGlobals = [ ];
              maxVersions = { };
              dmabufAllow = [ ];
              dmabufDeny = [ ];
              defaults = {
                acceleratedRendering = "deny";
                clipboardBoundary = "deny";
                highRisk = "deny";
                appDefaults = "deny";
                offDefaults = "deny";
                unclassified = "deny";
              };
            };
          };
          display-wayland-session = {
            type = "display-wayland.d2bus.org.WaylandSession";
            metadata.ownerRef = "Guest/corp-vm";
            spec = {
              guestRef = "Guest/corp-vm";
              hostRef = "Host/host-system";
              userRef = "User/alice";
              policyRef =
                "display-wayland.d2bus.org.WaylandPolicy/display-wayland-policy";
              identity = {
                label = "acceptance";
                activeColor = "#00ff00";
                inactiveColor = "#808080";
                urgentColor = "#ff0000";
                borderEnabled = true;
                borderWidth = 2;
                labelEnabled = true;
                labelText = "acceptance";
                labelPosition = "top-left";
              };
              crossDomainTrusted = true;
              reconnectGeneration = 1;
              virglVideo = false;
              filter = {
                allowGlobals = [ ];
                denyGlobals = [ ];
                maxVersions = { };
                dmabufAllow = [ ];
                dmabufDeny = [ ];
                debugLogging = false;
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
    machine.wait_for_file("/run/d2b/public.sock")

    # The nested VM must be able to create a TAP before an inner Cloud
    # Hypervisor process can boot. Probe that host capability explicitly and
    # report a concrete block instead of waiting for a dead runner.
    guest_fixture = machine.succeed(
        "if test -d /nix/store && test -d /var/lib/d2b/vms && "
        "runuser -u d2bd -- ip tuntap add d2b-guest-probe mode tap 2>/dev/null; "
        "then runuser -u d2bd -- ip tuntap del d2b-guest-probe mode tap 2>/dev/null; echo ready; "
        "else echo blocked-host; fi"
    ).strip()
    if guest_fixture != "ready":
        print(
            "BLOCKED: Cloud Hypervisor guest adoption requires nested TAP and "
            "delegated cgroup host posture; this runNixOSTest image does not "
            "provide it."
        )
        machine.succeed("runuser -u alice -- d2b auth status --json >/dev/null")
    else:
        machine.succeed(
            "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
            "d2b --zone work --json "
            "resource reconcile Guest/corp-vm"
        )
        machine.wait_until_succeeds(
            "jq -e '.entries[] | select(.vm == \"corp-vm\" and .role == \"ch-runner\")' "
            "/var/lib/d2b/daemon-state/pidfd-table.json"
        )
        runner = machine.succeed(
            "jq -r '.entries[] | select(.vm == \"corp-vm\" and .role == \"ch-runner\") "
            "| \"\\(.pid) \\(.startTimeTicks)\"' "
            "/var/lib/d2b/daemon-state/pidfd-table.json"
        ).strip()
        runner_pid, runner_start = runner.split()
        machine.succeed(f"test -d /proc/{runner_pid}")
        machine.succeed(
            f"test \"$(awk '{{print $22}}' /proc/{runner_pid}/stat)\" = {runner_start}"
        )
        machine.succeed(
            "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
            "d2b --zone work --json resource list Guest "
            "> /run/d2b-guest-before.json"
        )
        machine.succeed(
            "jq -e '.resources[] | select(.resourceRef == \"Guest/corp-vm\")' "
            "/run/d2b-guest-before.json"
        )

        machine.succeed("systemctl restart d2bd.service")
        machine.wait_for_unit("d2bd.service")
        machine.wait_for_file("/run/d2b/public.sock")
        machine.succeed(
            "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
            "d2b --zone work --json "
            "resource reconcile Guest/corp-vm | jq -e "
            "'.effect == \"cloud-hypervisor-adopted\"'"
        )
        machine.succeed(f"test -d /proc/{runner_pid}")
        machine.succeed(
            f"test \"$(awk '{{print $22}}' /proc/{runner_pid}/stat)\" = {runner_start}"
        )
        machine.wait_until_succeeds(
            f"jq -e '.entries[] | select(.vm == \"corp-vm\" and .role == \"ch-runner\" "
            f"and .pid == ({runner_pid}|tonumber) and "
            f".startTimeTicks == ({runner_start}|tonumber))' "
            "/var/lib/d2b/daemon-state/pidfd-table.json"
        )
  '';
}
