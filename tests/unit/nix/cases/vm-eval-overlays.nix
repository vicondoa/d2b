# Per-VM evaluations must inherit the host's nixpkgs overlays. Security-fix
# overlays are declared once on the host but must affect every VM closure too.
{ mkEval, ... }:

let
  fixture = { lib, ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.users.alice = { isNormalUser = true; uid = 1000; };

    nixpkgs.overlays = [
      (_final: _prev: {
        overlayProbeText = "guest-overlay-visible";
      })
    ];

    d2b.site = {
      waylandUser = "alice";
      launcherUsers = [ "alice" ];
      yubikey.enable = false;
    };
    d2b.acceptDestructiveV2Cutover = true;
    d2b.realms.work = {
      path = "work";
      placement = "host-local";
      broker = {
        enable = true;
        hostMutation = true;
      };
      network = {
        mode = "declared";
        lanSubnet = "10.20.0.0/24";
        uplinkSubnet = "192.0.2.0/30";
      };
      providers.runtime = {
        type = "runtime";
        implementationId = "cloud-hypervisor";
      };
      workloads.demo = {
        providerRefs.runtime = "runtime";
        config = { lib, pkgs, ... }: {
          networking.hostName = lib.mkDefault "demo";
          users.users.alice = { isNormalUser = true; uid = 1000; };
          environment.etc."overlay-probe".text = pkgs.overlayProbeText;
        };
      };
    };
  };

  cfg = (mkEval [ fixture ]).config;
  workload = builtins.head
    (builtins.filter
      (row: row.workloadName == "demo")
      cfg.d2b._index.workloads.enabledList);
in
{
  "vm-eval-overlays/guest-inherits-host-overlays" = {
    expr =
      cfg.d2b._computedWorkloads.${workload.workloadId}
        .config.environment.etc."overlay-probe".text;
    expected = "guest-overlay-visible";
  };
}
