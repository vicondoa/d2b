# Type-G runNixOSTest: Zone-native Cloud Hypervisor Guest acceptance.
#
# This is the public host selector for the controller-owned Guest lifecycle.
# It requires the nested KVM posture and fails closed when the host cannot
# provide it; an environment block is not acceptance evidence.
{ pkgs, self }:

let
  inherit (pkgs) lib;
  d2bLib = import ./lib.nix {
    inherit self;
    inherit lib;
  };
  cloudHypervisorArtifact =
    d2bLib.mkRuntimeCloudHypervisorArtifact pkgs;
  volumeProviderArtifact = d2bLib.mkVolumeProviderArtifact pkgs;

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
  guestSystem = d2bLib.mkGuestSystem {
    inherit pkgs;
    name = "acceptance-guest";
  };
  artifacts = {
    runtime-cloud-hypervisor = {
      inherit (cloudHypervisorArtifact) package type catalog;
    };
    volume-acceptance-provider = {
      inherit (volumeProviderArtifact) package type catalog;
    };
    acceptance-system = {
      package = guestSystem.config.system.build.toplevel;
      type = "nixos-system";
    };
  };
in
pkgs.testers.runNixOSTest {
  name = "d2b-runtime-cloud-hypervisor-guest-preflight";

  nodes.machine = d2bLib.d2bDaemonNode {
    writableStore = true;
    extra = { ... }: {
      d2b.site.adminUsers = [ "alice" ];
      environment.systemPackages = with pkgs; [ iproute2 jq iputils procps ];
      d2b.artifacts = artifacts;
      d2b.guestSystems.work.acceptance-guest = guestSystem;
      d2b.zones.local-root.trustedPublishers.d2b-cloud-hypervisor.signingKey =
        cloudHypervisorArtifact.trustedPublisher.signingKey;
      d2b.zones.local-root.trustedPublishers.d2b-volume-acceptance.signingKey =
        volumeProviderArtifact.trustedPublisher.signingKey;
      d2b.zones.work.trustedPublishers.d2b-cloud-hypervisor.signingKey =
        cloudHypervisorArtifact.trustedPublisher.signingKey;
      d2b.zones.work.trustedPublishers.d2b-volume-acceptance.signingKey =
        volumeProviderArtifact.trustedPublisher.signingKey;
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
          acceptance-guest = {
            type = "Guest";
            spec = {
              providerRef = "Provider/runtime-cloud-hypervisor";
              systemArtifactId = "acceptance-system";
              defaultDomain = "system";
              allowedDomains = [ "system" ];
              budget = { };
              volumeAttachmentDefaults = [ ];
              networkAttachments = [ ];
              deviceAttachments = [ ];
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

    machine.succeed(
        "test -e /dev/kvm && test -r /dev/kvm && test -w /dev/kvm || "
        "{ echo 'required KVM capability unavailable: /dev/kvm' >&2; exit 1; }"
    )
    machine.succeed(
        "test -e /dev/vhost-net && test -r /dev/vhost-net && test -w /dev/vhost-net || "
        "{ echo 'required Cloud Hypervisor vhost capability unavailable: /dev/vhost-net' >&2; exit 1; }"
    )
    machine.succeed(
        "test -r /sys/fs/cgroup/cgroup.controllers || "
        "{ echo 'required cgroup v2 capability unavailable' >&2; exit 1; }"
    )
    machine.succeed(
        "for controller in cpu memory io pids cpuset; do "
        "grep -qw \"$controller\" /sys/fs/cgroup/cgroup.controllers || "
        "{ echo \"required cgroup controller unavailable: $controller\" >&2; exit 1; }; "
        "done"
    )
    machine.succeed(
        "test -d /sys/fs/cgroup/d2b.slice && "
        "grep -qw 'cpu' /sys/fs/cgroup/d2b.slice/cgroup.subtree_control && "
        "grep -qw 'memory' /sys/fs/cgroup/d2b.slice/cgroup.subtree_control && "
        "grep -qw 'pids' /sys/fs/cgroup/d2b.slice/cgroup.subtree_control || "
        "{ echo 'required delegated d2b.slice cgroup posture unavailable' >&2; exit 1; }"
    )

    machine.succeed(
        "test -r /etc/d2b/artifact-catalog.json && "
        "jq -e '"
        "(.guestSetupDescriptors | any(.[]; "
        ".zone == \"work\" and .guest == \"acceptance-guest\" and "
        ".providerArtifactId == \"runtime-cloud-hypervisor\" and "
        ".descriptor.providerRef == \"Provider/runtime-cloud-hypervisor\" and "
        ".descriptor.systemArtifactId == \"acceptance-system\" and "
        ".descriptor.childRoles == [\"vmm\", \"ch-api\", \"guest-control\", \"system\"])) and "
        "(.guestClosures | any(.[]; "
        ".zone == \"work\" and .guest == \"acceptance-guest\" and "
        ".artifactId == \"acceptance-system\" and (.closurePaths | length > 0) and "
        "(. as $guest | ($guest.closurePaths | index($guest.toplevel)) != null) and "
        ".storeView.mountPoint == \"/nix/store\" and "
        "(.storeView.root | endswith(\"/zones/work/guests/acceptance-guest/store-view\")) and "
        "(.vmm.binaryPath | endswith(\"/bin/cloud-hypervisor\"))))' "
        "/etc/d2b/artifact-catalog.json"
    )
    machine.succeed(
        "test -r /etc/d2b/closures/zones/work/acceptance-guest.json && "
        "jq -e '"
        ".schemaVersion == \"v3\" and .artifactId == \"acceptance-system\" and "
        "(.closurePaths | length > 0) and "
        "(. as $guest | ($guest.closurePaths | index($guest.toplevel)) != null) and "
        ".storeView.mountPoint == \"/nix/store\" and "
        ".storeView.sync == \"broker-store-sync\" and "
        "(.vmm.argv | index(\"--api-socket\")) != null' "
        "/etc/d2b/closures/zones/work/acceptance-guest.json"
    )
    machine.succeed(
        "jq -e '"
        ".resources | any(.[]; .type == \"Guest\" and "
        ".metadata.name == \"acceptance-guest\" and "
        ".spec.providerRef == \"Provider/runtime-cloud-hypervisor\" and "
        ".spec.systemArtifactId == \"acceptance-system\") and "
        "all(.[]; (tostring | contains(\"/nix/store/\") | not) and "
        "(tostring | contains(\"\\\"argv\\\"\") | not))' "
        "/etc/d2b/zones/work/resource-bundle.json"
    )

    machine.wait_until_succeeds(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "d2b --zone work --json resource list Guest "
        ">/run/d2b-guest-ready.json && "
        "jq -e '"
        "(.resources | map(select(.type == \"Guest\" and "
        ".metadata.name == \"acceptance-guest\"))) as $guests | "
        "($guests | length) == 1 and "
        "$guests[0].status.phase == \"Ready\" and "
        "$guests[0].status.observedGeneration == $guests[0].metadata.generation and "
        "$guests[0].status.resource.runtimeReady == true and "
        "$guests[0].status.resource.bootstrapReady == true and "
        "$guests[0].status.resource.activeProcessCount == 1' "
        "/run/d2b-guest-ready.json",
        timeout=180,
    )
    machine.wait_until_succeeds(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "d2b --zone work --json resource list Process "
        ">/run/d2b-process-ready.json && "
        "jq -e '"
        "([.resources[] | select(.type == \"Process\" and "
        ".metadata.name == \"acceptance-guest-vmm\" and "
        ".metadata.ownerRef == \"Guest/acceptance-guest\" and "
        ".spec.providerRef == \"Provider/system-minijail\" and "
        ".spec.executionRef == \"Host/host-system\" and "
        ".spec.processClass == \"worker\" and "
        ".spec.template == \"cloud-hypervisor-runner\" and "
        ".status.phase == \"Ready\" and "
        ".status.observedGeneration == .metadata.generation and "
        ".status.resource.adopted == false)] | length == 1) and "
        "([.resources[] | select(.type == \"Process\" and "
        ".metadata.ownerRef == \"Provider/runtime-cloud-hypervisor\" and "
        ".spec.providerRef == \"Provider/system-minijail\" and "
        ".spec.executionRef == \"Host/host-system\" and "
        ".spec.processClass == \"controller\" and "
        ".spec.template == \"cloud-hypervisor-controller\" and "
        ".status.phase == \"Ready\" and "
        ".status.observedGeneration == .metadata.generation)] | length == 1)' "
        "/run/d2b-process-ready.json",
        timeout=180,
    )
    machine.wait_until_succeeds(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "d2b --zone work --json resource list Endpoint "
        ">/run/d2b-endpoint-ready.json && "
        "jq -e '"
        "([.resources[] | select(.type == \"Endpoint\" and "
        ".metadata.ownerRef == \"Guest/acceptance-guest\" and "
        ".status.phase == \"Ready\" and "
        ".status.observedGeneration == .metadata.generation)] | length == 2)' "
        "/run/d2b-endpoint-ready.json",
        timeout=180,
    )
    machine.wait_until_succeeds(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "d2b --zone work --json resource list Volume "
        ">/run/d2b-volume-ready.json && "
        "jq -e '"
        "([.resources[] | select(.type == \"Volume\" and "
        ".metadata.name == \"acceptance-guest-system\" and "
        ".metadata.ownerRef == \"Guest/acceptance-guest\" and "
        ".spec.source.settings.kind == \"nix-closure\" and "
        ".spec.source.settings.systemArtifactId == \"acceptance-system\" and "
        ".status.phase == \"Ready\" and "
        ".status.observedGeneration == .metadata.generation)] | length == 1)' "
        "/run/d2b-volume-ready.json",
        timeout=180,
    )
    machine.wait_for_file(
        "/var/lib/d2b/zones/work/guests/acceptance-guest/acceptance-guest.sock"
    )
    machine.succeed(
        "test -S /var/lib/d2b/zones/work/guests/acceptance-guest/acceptance-guest.sock && "
        "test -d /var/lib/d2b/zones/work/guests/acceptance-guest/store-view"
    )

    runner = machine.succeed(
        "set -- $(for proc in /proc/[0-9]*; do "
        "exe=$(readlink \"$proc/exe\" 2>/dev/null || true); "
        "case \"$exe\" in */bin/cloud-hypervisor) "
        "cmd=$(tr '\\0' ' ' < \"$proc/cmdline\"); "
        "case \"$cmd\" in *--api-socket*acceptance-guest*) "
        "pid=''${proc#/proc/}; "
        "printf '%s %s ' \"$pid\" \"$(awk '{print $22}' \"$proc/stat\")\";; "
        "esac;; esac; done); "
        "test \"$#\" -eq 2; printf '%s %s' \"$1\" \"$2\""
    ).strip()
    runner_pid, runner_start = runner.split()
    machine.succeed(f"test -d /proc/{runner_pid}")
    machine.succeed(
        f"test \"$(awk '{{print $22}}' /proc/{runner_pid}/stat)\" = {runner_start}"
    )
    machine.succeed(
        f"tr '\\0' ' ' < /proc/{runner_pid}/cmdline | "
        "grep -F -- '--api-socket' | grep -F -- 'acceptance-guest'"
    )

    machine.succeed("systemctl restart d2bd.service")
    machine.wait_for_unit("d2bd.service")
    machine.wait_for_file("/run/d2b/public.sock")
    machine.succeed(f"test -d /proc/{runner_pid}")
    machine.succeed(
        f"test \"$(awk '{{print $22}}' /proc/{runner_pid}/stat)\" = {runner_start}"
    )
    machine.wait_until_succeeds(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "d2b --zone work --json resource list Process "
        ">/run/d2b-process-adopted.json && "
        "jq -e '"
        "([.resources[] | select(.type == \"Process\" and "
        ".metadata.name == \"acceptance-guest-vmm\" and "
        ".metadata.ownerRef == \"Guest/acceptance-guest\" and "
        ".status.phase == \"Ready\" and "
        ".status.resource.adopted == true)] | length == 1)' "
        "/run/d2b-process-adopted.json",
        timeout=180,
    )
    machine.succeed(
        "set -- $(for proc in /proc/[0-9]*; do "
        "exe=$(readlink \"$proc/exe\" 2>/dev/null || true); "
        "case \"$exe\" in */bin/cloud-hypervisor) "
        "cmd=$(tr '\\0' ' ' < \"$proc/cmdline\"); "
        "case \"$cmd\" in *--api-socket*acceptance-guest*) "
        "pid=''${proc#/proc/}; "
        "printf '%s %s ' \"$pid\" \"$(awk '{print $22}' \"$proc/stat\")\";; "
        "esac;; esac; done); "
        f"test \"$#\" -eq 2 && test \"$1\" = {runner_pid} && "
        f"test \"$2\" = {runner_start}"
    )

    guest_revision = machine.succeed(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "d2b --zone work --json resource list Guest "
        "| jq -er '.resources[] | select(.type == \"Guest\" and "
        ".metadata.name == \"acceptance-guest\") | .metadata.revision'"
    ).strip()
    machine.succeed(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        f"d2b --zone work --json resource delete Guest/acceptance-guest "
        f"--revision {guest_revision} "
        ">/run/d2b-guest-delete.json"
    )
    machine.succeed(
        "jq -e '.resource.metadata.deletionRequestedAt != null' "
        "/run/d2b-guest-delete.json"
    )
    machine.wait_until_succeeds(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "d2b --zone work --json resource list Guest "
        ">/run/d2b-guest-draining.json && "
        "jq -e 'any(.resources[]; .type == \"Guest\" and "
        ".metadata.name == \"acceptance-guest\" and "
        ".metadata.deletionRequestedAt != null)' "
        "/run/d2b-guest-draining.json",
        timeout=30,
    )
    machine.wait_until_succeeds(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "d2b --zone work --json resource list Guest "
        "| jq -e 'all(.resources[]; .metadata.name != \"acceptance-guest\")'",
        timeout=180,
    )
    machine.wait_until_succeeds(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "d2b --zone work --json resource list Process "
        "| jq -e 'all(.resources[]; .metadata.name != \"acceptance-guest-vmm\")'",
        timeout=180,
    )
    machine.succeed(
        "test ! -S /var/lib/d2b/zones/work/guests/acceptance-guest/acceptance-guest.sock"
    )
  '';
}
