# nix-unit cases migrated from tests/restart-policy-eval.sh.
#
# Regression for the framework restart-policy invariant: retired per-VM
# lifecycle services must remain absent (or, if reintroduced, carry
# top-level `restartIfChanged = false`). The daemon itself is allowed to
# restart on switch/update; VM runner survival is guarded by
# d2bd.service KillMode=process plus daemon adoption/reconciliation.
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
# d2bd is a strict value case too, but with the opposite policy: it may
# restart on update and must use KillMode=process so VM runners survive.
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
    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
    };
    d2b.envs.work = {
      lanSubnet = "10.20.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
  };

  fullVm = { lib, ... }: {
    d2b.observability.enable = true;
    d2b.vms.full-vm = {
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

  # d2bd daemon eval: forced on so the unit materialises regardless of
  # any allReady gate. The daemon may restart on switch/update; systemd only
  # terminates the main daemon process and the restarted daemon re-adopts
  # surviving runners.
  dCfg = (mkEval [ base ({ ... }: { d2b.daemonExperimental.enable = true; }) ]).config;
  dSvc = dCfg.systemd.services.d2bd or null;
in
{
  "restart-policy/d2b-template" = hostOk "d2b@";
  "restart-policy/microvm-template" = hostOk "microvm@";
  "restart-policy/microvm-virtiofsd-full-vm" = hostOk "microvm-virtiofsd@full-vm";
  "restart-policy/swtpm" = hostOk "d2b-full-vm-swtpm";
  "restart-policy/snd" = hostOk "d2b-full-vm-snd";
  "restart-policy/video" = hostOk "d2b-full-vm-video";
  "restart-policy/gpu" = hostOk "d2b-full-vm-gpu";
  "restart-policy/otel-relay-template" = hostOk "d2b-otel-relay@";
  "restart-policy/otel-host-bridge" = hostOk "d2b-otel-host-bridge";
  "restart-policy/ch-exporter" = hostOk "d2b-ch-exporter";
  "restart-policy/guest-otel-vsock-out" = guestOk "d2b-otel-vsock-out";
  "restart-policy/obs-otel-vsock-in-host" = obsOk "d2b-otel-vsock-in-host";

  "restart-policy/d2bd-present" = {
    expr = dSvc != null;
    expected = true;
  };
  "restart-policy/d2bd-restarts-on-update" = {
    expr = if dSvc != null then (dSvc.restartIfChanged or null) else null;
    expected = true;
  };
  "restart-policy/d2bd-killmode-process" = {
    expr = if dSvc != null then (dSvc.serviceConfig.KillMode or null) else null;
    expected = "process";
  };
}
