{ config, pkgs, lib, ... }:

let
  cfg = config.nixling;
in
{
  # No autostart from the upstream microvm.nix side — nixling owns
  # all autostart wantedBy bits on its own `nixling@<vm>.service`
  # wrapper template (see below). Setting `microvm.autostart = []`
  # ensures upstream-managed `microvm@<vm>.service` units do NOT
  # carry a `WantedBy=multi-user.target` of their own; the wrapper
  # is the single source of truth for boot starts.
  microvm.autostart = [ ];

  # ---------------------------------------------------------------------------
  # nixling@.service template — user-facing per-VM wrapper.
  #
  # `microvm@.service` is microvm.nix's template — it carries the runner
  # ExecStart, the virtiofsd dependency chain, the User=microvm setting,
  # all the integration knobs upstream maintains and tests. Replacing or
  # forking that template is fragile. Instead nixling adds its own
  # template that wraps it:
  #
  #   - BindsTo + After microvm@%i: if the underlying VM stops or
  #     restarts, the wrapper follows. (BindsTo handles only the
  #     bound→wrapper direction; the explicit ExecStop propagates the
  #     wrapper→bound direction so `systemctl stop nixling@<vm>` stops
  #     `microvm@<vm>` too.)
  #   - Explicit ExecStart so `systemctl start nixling@<vm>` boots the
  #     VM without relying on BindsTo start-pull side-effects.
  #   - Explicit ExecStop so `systemctl stop nixling@<vm>` propagates.
  #   - PropagatesStopTo (systemd ≥249) belt-and-suspenders on the
  #     stop direction.
  #   - Restart=on-failure intentionally OMITTED: the underlying
  #     `microvm@` unit owns restart policy (microvm.nix configures it
  #     per-VM). The BindsTo cycles the wrapper when microvm@ restarts.
  #
  # Per-instance overrides below (mapAttrs over cfg.vms) attach
  # `WantedBy=multi-user.target` for VMs with autostart=true, since
  # microvm.nix's upstream-managed autostart was disabled above.
  # ---------------------------------------------------------------------------
  systemd.services = ({
    "nixling@" = {
      description = "nixling: MicroVM %i";
      bindsTo  = [ "microvm@%i.service" ];
      after    = [ "microvm@%i.service" ];
      # wantedBy is set per-instance below for autostart=true VMs.
      wantedBy = [ ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = "${pkgs.systemd}/bin/systemctl start microvm@%i.service";
        ExecStop  = "${pkgs.systemd}/bin/systemctl stop  microvm@%i.service";
        # PropagatesStopTo (systemd ≥249) is a belt-and-suspenders for
        # the stop direction; works alongside ExecStop without conflict.
        PropagatesStopTo = [ "microvm@%i.service" ];
      };
    };
  })
  // (lib.mapAttrs' (name: _: lib.nameValuePair "nixling@${name}" {
    # Per-instance wantedBy for autostart=true VMs. The wrapper is the
    # single source of truth for boot-time starts now that
    # microvm.autostart = [] disables upstream-managed wantedBy on the
    # `microvm@` template.
    wantedBy = [ "multi-user.target" ];
  }) (lib.filterAttrs (_: vm: vm.enable && vm.autostart) cfg.vms));
}
