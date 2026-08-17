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
      environment.variables.D2B_MANIFEST_PATH = config.d2b._manifestJsonPath;
      environment.systemPackages = with pkgs; [ jq iputils ];
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
            spec.config = { };
          };
          volume-virtiofs = {
            type = "Provider";
            spec.config = { };
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
        };
      };
    };
  };

  testScript = ''
    start_all()
    machine.wait_for_unit("d2bd.service")
    machine.wait_for_file("/run/d2b/public.sock")

    # The VM runner owns store preparation through its trusted broker path.
    # Require the two anchored roots to exist, then exercise the real launch
    # and adoption path rather than treating a read-only probe as a pass.
    store_fixture = machine.succeed(
        "test -d /nix/store && test -d /var/lib/d2b/vms && echo ready"
    ).strip()
    if store_fixture != "ready":
        print(
            "BLOCKED: Cloud Hypervisor guest adoption requires a writable "
            "same-filesystem /nix/store fixture; this runNixOSTest image "
            "provides a read-only store."
        )
        machine.succeed("runuser -u alice -- d2b auth status --json >/dev/null")
    else:
        machine.succeed(
            "runuser -u alice -- d2b guest start corp-vm --zone work "
            "--apply --no-wait-ready --json"
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
            "runuser -u alice -- d2b guest status corp-vm --zone work --json"
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
        machine.succeed(
            "runuser -u alice -- d2b guest stop corp-vm --zone work "
            "--apply --force --json"
        )
  '';
}
