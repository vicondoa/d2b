# Type-G runNixOSTest: authenticated Resource operator and framework census.
#
# This fixture is intentionally separate from the native controller canaries:
# it reaches the installed d2b CLI, public socket, systemd restart boundary,
# and the framework-declared daemon unit surface in a real NixOS guest. The
# census does not sweep every d2b-prefixed unit on an operator host, because
# optional or managed infrastructure is outside this fixture's ownership.
{ pkgs, self }:

let
  inherit (pkgs) lib;
  d2bLib = import ./lib.nix {
    inherit self;
    inherit lib;
  };
  cloudHypervisorArtifact =
    d2bLib.mkRuntimeCloudHypervisorArtifact pkgs;
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
  providerArtifact = d2bLib.mkAcceptanceProviderArtifact pkgs;
  acceptancePublisherKey = providerArtifact.trustedPublisher.signingKey;
  volumeProviderArtifact = d2bLib.mkVolumeProviderArtifact pkgs;
  acceptanceGuestSystem = d2bLib.mkGuestSystem {
    inherit pkgs;
    name = "acceptance-guest";
  };
  netVmSystem = d2bLib.mkGuestSystem {
    inherit pkgs;
    name = "acceptance-net-vm";
  };
  artifacts = {
    acceptance-provider = {
      inherit (providerArtifact) package type catalog;
    };
    runtime-cloud-hypervisor = {
      inherit (cloudHypervisorArtifact) package type catalog;
    };
    volume-acceptance-provider = {
      inherit (volumeProviderArtifact) package type catalog;
    };
    acceptance-system = {
      package = acceptanceGuestSystem.config.system.build.toplevel;
      type = "nixos-system";
    };
    net-vm-base = {
      package = netVmSystem.config.system.build.toplevel;
      type = "nixos-system";
    };
  };
  hostRuntime = pkgs.writeText "d2b-acceptance-host-runtime.json" (builtins.toJSON {
    schemaVersion = "v1";
    bundleVersion = 1;
    generatedAt = "1970-01-01T00:00:00.000Z";
    nftAppliedHash = null;
    ifnames = [ ];
  });
in
pkgs.testers.runNixOSTest {
  name = "d2b-resource-operator-activation";

  nodes.machine = d2bLib.d2bDaemonNode {
      writableStore = true;
      extra = { ... }: {
        boot.kernelModules = [ "br_netfilter" "tpm_vtpm_proxy" ];
        networking.nftables.enable = true;
        networking.nftables.ruleset = lib.mkAfter ''
          table inet d2b {}
        '';
        systemd.tmpfiles.rules = [
          "d /etc/NetworkManager/conf.d 0755 root root -"
        ];
        environment.etc."d2b/acceptance-host-runtime.json".source = hostRuntime;
        d2b.site.adminUsers = [ "alice" ];
        systemd.services.d2bd.serviceConfig.ExecStartPre = lib.mkAfter [
          "+${pkgs.writeShellScript "d2b-acceptance-hosts-prep" ''
            if [ -L /etc/hosts ]; then
              ${pkgs.coreutils}/bin/cat /etc/hosts > /run/d2b-acceptance-hosts
              ${pkgs.coreutils}/bin/rm -f /etc/hosts
              ${pkgs.coreutils}/bin/install -o root -g root -m 0644 \
                /run/d2b-acceptance-hosts /etc/hosts
            fi
          ''}"
          "+${pkgs.writeShellScript "d2b-acceptance-host-runtime-prep" ''
            ${pkgs.coreutils}/bin/install -D -o root -g d2bd -m 0640 \
              /etc/d2b/acceptance-host-runtime.json \
              /var/lib/d2b/runtime/host-runtime.json
          ''}"
          "+${pkgs.writeShellScript "d2b-acceptance-cgroup-prep" ''
            relative=$(sed -n 's/^0:://p' /proc/self/cgroup)
            path="/sys/fs/cgroup''${relative}/cgroup.kill"
            if [ -e "$path" ]; then
              chown d2bd:d2bd "$path" 2>/dev/null || true
              chmod u+w "$path" 2>/dev/null || true
            fi
          ''}"
        ];
        users.users.bob = {
          isNormalUser = true;
          uid = 1001;
        };
        d2b.artifacts = artifacts;
      d2b.guestSystems.work.acceptance-guest = acceptanceGuestSystem;
      d2b.zones.local-root.trustedPublishers.d2b-u20-acceptance.signingKey =
        acceptancePublisherKey;
      d2b.zones.local-root.trustedPublishers.d2b-volume-acceptance.signingKey =
        volumeProviderArtifact.trustedPublisher.signingKey;
      d2b.zones.local-root.trustedPublishers.d2b-cloud-hypervisor.signingKey =
        cloudHypervisorArtifact.trustedPublisher.signingKey;
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
      d2b.zones.work.trustedPublishers.d2b-u20-acceptance.signingKey =
        acceptancePublisherKey;
      d2b.zones.work.trustedPublishers.d2b-volume-acceptance.signingKey =
        volumeProviderArtifact.trustedPublisher.signingKey;
      d2b.zones.work.trustedPublishers.d2b-cloud-hypervisor.signingKey =
        cloudHypervisorArtifact.trustedPublisher.signingKey;
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
        acceptance-host = {
          type = "Provider";
          spec = {
            artifactId = "acceptance-provider";
            config = { };
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
              artifactId = "volume-acceptance-provider";
              config = {
                controllerExecutionRef = "Host/host-system";
                sourcePolicies = [
                  {
                    id = "default-state";
                    class = "local-path";
                    volumeKinds = [ "durable" "state" "cache" ];
                  }
                ];
              };
            };
          };
          volume-virtiofs = {
            type = "Provider";
            spec = {
              artifactId = "volume-acceptance-provider";
              config = {
                controllerExecutionRef = "Host/host-system";
              };
            };
          };
          network-local = {
            type = "Provider";
            spec = {
              artifactId = "acceptance-provider";
              config.controllerExecutionRef = "Host/host-system";
            };
          };
          device-tpm = {
            type = "Provider";
            spec = {
              artifactId = "acceptance-provider";
              config = {
                controllerExecutionRef = "Host/host-system";
                logLevel = 20;
              };
            };
          };
          runtime-cloud-hypervisor = {
            type = "Provider";
            spec = {
              artifactId = "runtime-cloud-hypervisor";
              config = cloudHypervisorConfig;
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
          clipboard-wayland = {
            type = "Provider";
            spec = {
              artifactId = "acceptance-provider";
              config = {
                hostExecutionRef = "Host/host-system";
                hostUserRef = "User/alice";
                displayWaylandRef = "Provider/display-wayland";
                guestSources = [ { guestRef = "Guest/acceptance-guest"; } ];
              };
            };
          };
          notification-desktop = {
            type = "Provider";
            spec = {
              artifactId = "acceptance-provider";
              config = {
                hostExecutionRef = "Host/host-system";
                hostUserRef = "User/alice";
                displayWaylandRef = "Provider/display-wayland";
                guestSources = [
                  {
                    guestRef = "Guest/acceptance-guest";
                    categories = [ "system.info" ];
                  }
                ];
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
            metadata.ownerRef = "Guest/acceptance-guest";
            spec = {
              guestRef = "Guest/acceptance-guest";
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
          acceptance-volume = {
            type = "Volume";
            metadata.ownerRef = "Guest/acceptance-guest";
            spec = {
              attachments = [ ];
              providerRef = "Provider/volume-local";
              source = {
                executionRef = "Host/host-system";
                settings = {
                  kind = "local-path";
                  sourcePolicyId = "default-state";
                };
              };
              kind = "state";
              layout = [
                {
                  path = "";
                  type = "directory";
                  ownerRef = "User/alice";
                  groupRef = "User/alice";
                  mode = "0700";
                  accessAcl = [ ];
                  defaultAcl = [ ];
                  adoptionPolicy = "adopt-with-live-owner-proof";
                  foreignChildPolicy = "preserve";
                  invariants = [ "scope-authorization-required" ];
                  leaseClass = "none";
                  noFollow = true;
                  recursive = false;
                  createPolicy = "create-if-never-provisioned";
                  repairPolicy = "exact-owner";
                  cleanupPolicy = "owner-controlled";
                  restartPolicy = "preserve-across-controller-restart";
                  sensitivity = "private";
                }
              ];
              views.controller = {
                path = "";
                rights = [ "read" "write" "create" "delete" "traverse" ];
              };
            };
          };
          acceptance-network = {
            type = "Network";
            spec = {
              providerRef = "Provider/network-local";
              netVmSystemArtifactId = "net-vm-base";
              lanCidr = "10.40.0.0/24";
              uplinkCidr = "192.0.2.4/30";
              dhcp = {
                domain = null;
                ignoreClientNames = true;
              };
              dns = {
                cacheSize = 1000;
                forwarders = [ ];
              };
              mdns = {
                dnsmasqLocal = false;
                dnsmasqLocalPort = 53530;
                enable = false;
                publishWorkstation = false;
                reflector = true;
              };
              mssClamp = false;
              routing.hostBlocklist = [
                "10.0.0.0/8"
                "169.254.0.0/16"
                "172.16.0.0/12"
                "192.168.0.0/16"
              ];
              attachments = [
                {
                  executionRef = "Guest/acceptance-guest";
                  index = 10;
                }
              ];
              isolation.allowEastWest = false;
            };
          };
          acceptance-tpm = {
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
          acceptance-guest = {
            type = "Guest";
            spec = {
              providerRef = "Provider/runtime-cloud-hypervisor";
              systemArtifactId = "acceptance-system";
              defaultDomain = "system";
              allowedDomains = [ "system" ];
              budget = { };
              volumeAttachmentDefaults = [ ];
              networkAttachments = [
                {
                  networkRef = "Network/acceptance-network";
                  default = true;
                }
              ];
              deviceAttachments = [
                {
                  deviceRef = "Device/acceptance-tpm";
                  exclusive = true;
                }
              ];
            };
          };
        };
        environment.systemPackages = [ pkgs.jq ];
      };
  };

  testScript = ''
    start_all()
    machine.wait_for_unit("nftables.service")
    machine.succeed("nft list table inet d2b")
    machine.wait_for_unit("d2b-broker.socket")
    machine.wait_for_unit("d2bd.service")
    machine.wait_for_file("/run/d2b/public.sock")
    machine.succeed("runuser -u alice -- d2b auth status --json >/run/d2b-auth-before.json")

    # The authenticated operator path must reach the public Resource API for
    # every Wave 6 type and observe committed desired state, not just an empty
    # list response.
    for resource_type in ["Volume", "Network", "Device", "Guest"]:
        path = f"/run/d2b-resource-{resource_type.lower()}-before.json"
        machine.succeed(
            f"runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
            f"d2b --zone work --json resource list "
            f"{resource_type} >{path}"
        )
        machine.succeed(
            f"jq -e '.snapshotRevision > 0 and "
            f"(.resources | length >= 1) and "
            f"any(.resources[]; .type == \"{resource_type}\")' "
            f"{path}"
        )

    machine.succeed(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "d2b --zone work --json resource list Provider "
        ">/run/d2b-providers-before.json"
    )
    machine.succeed(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "d2b --zone work --json resource list Provider "
        ">/run/d2b-providers-after.json && "
        "jq -e '.resources | "
        "map(select(.type == \"Provider\" and "
        "(.metadata.name == \"display-wayland\" or "
        ".metadata.name == \"clipboard-wayland\" or "
        ".metadata.name == \"notification-desktop\"))) | "
        "length == 3' /run/d2b-providers-after.json"
    )
    machine.succeed(
        "jq -e '"
        "(.resources | map(select(.type == \"Provider\" and .metadata.name == \"display-wayland\")) | length == 1) and "
        "(.resources | map(select(.type == \"Provider\" and .metadata.name == \"clipboard-wayland\")) | length == 1) and "
        "(.resources | map(select(.type == \"Provider\" and .metadata.name == \"notification-desktop\")) | length == 1) and "
        "(.resources[] | select(.type == \"Provider\" and .metadata.name == \"display-wayland\") | "
        ".spec.config.runtimeVolumePolicyId == \"display-wayland.wlproxy-runtime.v1\") and "
        "(.resources[] | select(.type == \"Provider\" and .metadata.name == \"clipboard-wayland\") | ("
        ".spec.config.hostExecutionRef == \"Host/host-system\" and "
        ".spec.config.hostUserRef == \"User/alice\" and "
        ".spec.config.displayWaylandRef == \"Provider/display-wayland\" and "
        "(.spec.config.guestSources | length == 1) and "
        ".spec.config.guestSources[0].guestRef == \"Guest/acceptance-guest\")) and "
        "(.resources[] | select(.type == \"Provider\" and .metadata.name == \"notification-desktop\") | ("
        ".spec.config.hostExecutionRef == \"Host/host-system\" and "
        ".spec.config.hostUserRef == \"User/alice\" and "
        ".spec.config.displayWaylandRef == \"Provider/display-wayland\" and "
        "(.spec.config.guestSources | length == 1) and "
        ".spec.config.guestSources[0].guestRef == \"Guest/acceptance-guest\" and "
        ".spec.config.guestSources[0].categories == [\"system.info\"]))' "
        "/run/d2b-providers-before.json"
    )
    persistent_refs = [
        "Volume/acceptance-volume",
        "Network/acceptance-network",
        "Device/acceptance-tpm",
        "Guest/acceptance-guest",
    ]
    # Guest launch and restart adoption are owned by the dedicated
    # runtime-cloud-hypervisor-guest-preflight VM check.
    reconcile_refs = persistent_refs[:-1]
    for resource_ref in reconcile_refs:
        resource_type, resource_name = resource_ref.split("/", 1)
        safe_name = resource_ref.replace("/", "-").lower()
        machine.succeed(
            f"jq -e '.resources[] | select(.type == \"{resource_type}\" and "
            f".metadata.name == \"{resource_name}\") "
            f"| (.metadata.uid != null and .metadata.generation > 0 and "
            f".metadata.revision > 0)' "
            f"/run/d2b-resource-{resource_ref.split('/')[0].lower()}-before.json"
        )
        machine.succeed(
            f"runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
            f"d2b --zone work --json resource reconcile {resource_ref} "
            f">/run/d2b-reconcile-{safe_name}.json "
            f"2>/run/d2b-reconcile-{safe_name}.stderr || "
            f"(cat /run/d2b-reconcile-{safe_name}.stderr; "
            f"cat /run/d2b-reconcile-{safe_name}.json; "
            f"exit 1)"
        )
        expected_effect = {
            "Volume": "storage-scope-reconciled",
            "Network": "network-bridge-reconciled",
            "Device": "device-tpm-reconciled",
        }[resource_ref.split("/", 1)[0]]
        machine.succeed(
            f"jq -e '.ready == true and .authenticated == true and "
            f".resourceRef == \"{resource_ref}\" and "
            f".effect == \"{expected_effect}\" and "
            f"(.providerRef | startswith(\"Provider/\"))' "
            f"/run/d2b-reconcile-{safe_name}.json"
        )
        if resource_type == "Device":
            machine.succeed("test -S /run/d2b/vms/acceptance-guest/tpm.sock")
        if resource_type == "Network":
            machine.succeed(
                "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
                "d2b --zone work --json resource list Network "
                ">/run/d2b-network-after-reconcile.json"
            )
            machine.succeed(
                "jq -e '"
                ".resources[] | select(.type == \"Network\" and "
                ".metadata.name == \"acceptance-network\") | "
                "(.status.phase == \"Ready\" and "
                ".status.observedGeneration == .metadata.generation and "
                ".status.resource.configVolume.phase == \"Ready\" and "
                ".status.resource.netVm.phase == \"Ready\" and "
                ".status.resource.volumeAttachment.phase == \"Ready\")' "
                "/run/d2b-network-after-reconcile.json"
            )
    machine.fail(
        "runuser -u bob -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "d2b --zone work --json resource list Volume "
        ">/run/d2b-unauthorized-resource.log 2>&1"
    )
    machine.wait_until_succeeds(
        "journalctl -u d2bd.service --no-pager | grep -Eq "
        "'interaction_runtime_ready[=: ]+true'",
        timeout=60,
    )
    machine.succeed(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "d2b --zone work --json resource list Host "
        ">/run/d2b-host-before.json && "
        "jq -e '.resources[] | select(.type == \"Host\" and .metadata.name == \"host-system\") "
        "| .status.phase == \"Ready\"' /run/d2b-host-before.json"
    )

    machine.succeed("systemctl restart d2bd.service")
    machine.wait_for_unit("d2bd.service")
    machine.wait_for_file("/run/d2b/public.sock")
    machine.wait_until_succeeds(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "d2b --zone work --json resource list Volume >/dev/null",
        timeout=60,
    )
    machine.succeed("runuser -u alice -- d2b auth status --json >/run/d2b-auth-after.json")
    for resource_type in ["Volume", "Network", "Device", "Guest"]:
        path = f"/run/d2b-resource-{resource_type.lower()}-after.json"
        machine.succeed(
            f"runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
            f"d2b --zone work --json resource list "
            f"{resource_type} >{path}"
        )
        machine.succeed(
            f"jq -e '.snapshotRevision > 0 and "
            f"(.resources | length >= 1) and "
            f"any(.resources[]; .type == \"{resource_type}\")' "
            f"{path}"
        )

    machine.succeed(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "d2b --zone work --json resource list Provider "
        ">/run/d2b-providers-after.json && "
        "jq -e '.resources | "
        "map(select(.type == \"Provider\" and "
        "(.metadata.name == \"display-wayland\" or "
        ".metadata.name == \"clipboard-wayland\" or "
        ".metadata.name == \"notification-desktop\"))) | "
        "length == 3' /run/d2b-providers-after.json"
    )
    for resource_ref in persistent_refs:
        resource_type, resource_name = resource_ref.split("/", 1)
        safe_name = resource_ref.replace("/", "-").lower()
        machine.succeed(
            f"jq -e --arg ref '{resource_ref}' "
            f"--slurpfile before /run/d2b-resource-{resource_type.lower()}-before.json "
            f"'.resources[] | select(.type == \"{resource_type}\" and "
            f".metadata.name == \"{resource_name}\") as $after | "
            f"($before[0].resources[] | select(.type == \"{resource_type}\" and "
            f".metadata.name == \"{resource_name}\")) as $old | "
            f"($after.metadata.uid == $old.metadata.uid and "
            f"$after.metadata.generation == $old.metadata.generation and "
            f"$after.metadata.revision >= $old.metadata.revision)' "
            f"/run/d2b-resource-{resource_type.lower()}-after.json"
        )
    machine.succeed(
        "jq -e '"
        "(.resources | map(select(.type == \"Provider\" and .metadata.name == \"display-wayland\")) | length == 1) and "
        "(.resources | map(select(.type == \"Provider\" and .metadata.name == \"clipboard-wayland\")) | length == 1) and "
        "(.resources | map(select(.type == \"Provider\" and .metadata.name == \"notification-desktop\")) | length == 1) and "
        "(.resources[] | select(.type == \"Provider\" and .metadata.name == \"display-wayland\") | "
        ".spec.config.runtimeVolumePolicyId == \"display-wayland.wlproxy-runtime.v1\") and "
        "(.resources[] | select(.type == \"Provider\" and .metadata.name == \"clipboard-wayland\") | ("
        ".spec.config.hostExecutionRef == \"Host/host-system\" and "
        ".spec.config.hostUserRef == \"User/alice\" and "
        ".spec.config.displayWaylandRef == \"Provider/display-wayland\" and "
        "(.spec.config.guestSources | length == 1) and "
        ".spec.config.guestSources[0].guestRef == \"Guest/acceptance-guest\")) and "
        "(.resources[] | select(.type == \"Provider\" and .metadata.name == \"notification-desktop\") | ("
        ".spec.config.hostExecutionRef == \"Host/host-system\" and "
        ".spec.config.hostUserRef == \"User/alice\" and "
        ".spec.config.displayWaylandRef == \"Provider/display-wayland\" and "
        "(.spec.config.guestSources | length == 1) and "
        ".spec.config.guestSources[0].guestRef == \"Guest/acceptance-guest\" and "
        ".spec.config.guestSources[0].categories == [\"system.info\"]))' "
        "/run/d2b-providers-after.json"
    )
    machine.succeed(
        "jq -e '"
        ".resources[] | select(.type == \"Network\" and "
        ".metadata.name == \"acceptance-network\") | "
        "(.status.phase == \"Ready\" and "
        ".status.observedGeneration == .metadata.generation and "
        ".status.resource.configVolume.phase == \"Ready\" and "
        ".status.resource.netVm.phase == \"Ready\" and "
        ".status.resource.volumeAttachment.phase == \"Ready\")' "
        "/run/d2b-resource-network-after.json"
    )
    machine.succeed(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "d2b --zone work --json resource list Host "
        ">/run/d2b-host-after.json && "
        "jq -e '.resources[] | select(.type == \"Host\" and .metadata.name == \"host-system\") "
        "| .status.phase == \"Ready\"' /run/d2b-host-after.json"
    )

    declared = set(
        machine.succeed("cat /etc/d2b/daemon-acceptance-units").split()
    )
    required = {
        "d2bd.service",
        "d2b-broker.socket",
        "d2b-broker.service",
    }
    assert declared == required, (
        f"unexpected framework acceptance census: {declared}"
    )
    unit_names = set(
        machine.succeed(
            "systemctl list-units --no-pager --all --plain "
            "| awk '{print $1}' | sort"
        ).split()
    )
    assert required <= unit_names, (
        f"framework daemon units missing: {required - unit_names}"
    )

    # Provider packages are code loaded by d2bd, never framework-declared
    # persistent services. Optional or managed host units are outside this
    # fixture's census.
    provider_units = sorted(
        unit
        for unit in declared
        if "provider" in unit and (unit.endswith(".service") or unit.endswith(".socket"))
    )
    assert not provider_units, f"Provider-owned persistent units found: {provider_units}"
  '';
}
