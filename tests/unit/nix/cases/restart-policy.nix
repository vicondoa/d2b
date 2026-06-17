# nix-unit cases migrated from tests/restart-policy-eval.sh.
#
# Regression for the framework restart-policy invariant: every per-VM
# lifecycle service AND the long-lived nixlingd supervisor must carry
# top-level `restartIfChanged = false` so a `nixos-rebuild switch` never
# cycles a running VM/runner DAG mid-flight (which would terminate
# cloud-hypervisor, evaporate in-RAM Entra device-bound tokens, and drop
# the user's session).
#
# Synthesizes one workload VM with graphics + audio + TPM + observability
# all enabled so EVERY per-VM lifecycle service would materialise in a
# single eval, then introspects the real host `systemd.services` via
# `mkEval`.
#
# Faithful note on the per-VM/observability host units: in the daemon-only
# end-state those units are RETIRED — their replacements are broker
# `SpawnRunner` runners whose restart contract is owned by the broker's
# pidfd supervisor, not a `restartIfChanged` knob. The bash gate handled
# them with `check_optional` (SKIP when absent, PASS when present with
# `restartIfChanged = false`). Each is migrated to the `ricOkOrAbsent`
# invariant below: it passes while the unit is absent and would fail only
# if such a unit were RE-INTRODUCED with a missing/true restart policy —
# exactly the regression the bash retained these checks to guard against.
# The single hard assertion (nixlingd carries restartIfChanged = false) is
# preserved as a strict value case.
{ mkEval, ... }:

let
  base = { lib, ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };
    nixling.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
    };
    nixling.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
  };

  fullVm = { lib, ... }: {
    nixling.observability.enable = true;
    nixling.vms.full-vm = {
      enable = true;
      env = "work";
      index = 10;
      ssh.user = "alice";
      graphics.enable = true;
      audio.enable = true;
      tpm.enable = true;
      observability.enable = true;
      config = { lib, ... }: {
        networking.hostName = lib.mkDefault "full-vm";
        users.users.alice = { isNormalUser = true; uid = 1000; };
      };
    };
  };

  rpCfg = (mkEval [ base fullVm ]).config;
  hostSvcs = rpCfg.systemd.services;
  guestSvcs =
    if rpCfg ? microvm && rpCfg.microvm ? vms && rpCfg.microvm.vms ? "full-vm"
    then rpCfg.microvm.vms."full-vm".config.config.systemd.services
    else { };
  obsGuestSvcs =
    if rpCfg ? microvm && rpCfg.microvm ? vms && rpCfg.microvm.vms ? "sys-obs"
    then rpCfg.microvm.vms."sys-obs".config.config.systemd.services
    else { };

  # Faithful successor of the bash `check_optional`: a unit passes when it
  # is absent (SKIP) OR present carrying `restartIfChanged = false` (PASS).
  ricOkOrAbsent = svcs: key:
    !(builtins.hasAttr key svcs)
    || (svcs.${key}.restartIfChanged or null) == false;

  hostOk = key: { expr = ricOkOrAbsent hostSvcs key; expected = true; };
  guestOk = key: { expr = ricOkOrAbsent guestSvcs key; expected = true; };
  obsOk = key: { expr = ricOkOrAbsent obsGuestSvcs key; expected = true; };

  # nixlingd daemon eval: forced on so the unit materialises regardless of
  # any allReady gate. It is the long-lived supervisor whose pidfd owns the
  # child-runner DAG; a rebuild-triggered restart would tear down every
  # in-flight VM process, so the VM lifecycle policy extends to the daemon.
  dCfg = (mkEval [ base ({ ... }: { nixling.daemonExperimental.enable = true; }) ]).config;
  dSvc = dCfg.systemd.services.nixlingd or null;
in
{
  "restart-policy/nixling-template" = hostOk "nixling@";
  "restart-policy/microvm-template" = hostOk "microvm@";
  "restart-policy/microvm-virtiofsd-full-vm" = hostOk "microvm-virtiofsd@full-vm";
  "restart-policy/swtpm" = hostOk "nixling-full-vm-swtpm";
  "restart-policy/snd" = hostOk "nixling-full-vm-snd";
  "restart-policy/video" = hostOk "nixling-full-vm-video";
  "restart-policy/gpu" = hostOk "nixling-full-vm-gpu";
  "restart-policy/otel-relay-template" = hostOk "nixling-otel-relay@";
  "restart-policy/otel-host-bridge" = hostOk "nixling-otel-host-bridge";
  "restart-policy/ch-exporter" = hostOk "nixling-ch-exporter";
  "restart-policy/guest-otel-vsock-out" = guestOk "nixling-otel-vsock-out";
  "restart-policy/obs-otel-vsock-in-host" = obsOk "nixling-otel-vsock-in-host";

  "restart-policy/nixlingd-present" = {
    expr = dSvc != null;
    expected = true;
  };
  "restart-policy/nixlingd-no-restart" = {
    expr = if dSvc != null then (dSvc.restartIfChanged or null) else null;
    expected = false;
  };
}
