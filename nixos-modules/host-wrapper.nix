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

  # v0.1.3 fix: `microvm.autostart = []` above is NOT sufficient.
  # Upstream microvm.nix unconditionally emits
  # `systemd.targets.microvms.wants = ["microvm@<vm>.service" …]`
  # for every `microvm.vms.<vm>`, and `microvms.target` itself is
  # wantedBy multi-user.target. The cascade pulls in microvm@<vm>
  # at boot regardless of `microvm.autostart`.
  #
  # Override `microvms.target` to ONLY want the autostart=true VMs
  # (which already go through the `nixling@<vm>` wrapper via
  # `multi-user.target.wants`). Workload VMs (autostart=false) are
  # then started exclusively on-demand via `nixling up <vm>`.
  systemd.targets.microvms.wants = lib.mkForce
    (map (name: "microvm@${name}.service")
      (lib.attrNames (lib.filterAttrs (_: vm: vm.enable && vm.autostart) cfg.vms)));

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
  systemd.services = {
    "nixling@" = {
      description = "nixling: MicroVM %i";
      bindsTo  = [ "microvm@%i.service" ];
      after    = [ "microvm@%i.service" ];
      # wantedBy stays empty on the template itself; per-instance
      # autostart is wired via `systemd.targets.multi-user.wants`
      # below to avoid emitting per-instance unit FILES that would
      # shadow the template (NixOS's `systemd.services."nixling@${name}"`
      # generates a separate file, not a drop-in — and a separate
      # file means the template's ExecStart/ExecStop are NOT
      # inherited, breaking systemd unit validation).
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
  };

  # Wire autostart=true VMs into multi-user.target via target.wants
  # symlinks instead of per-instance unit-file overrides. systemd
  # then resolves `nixling@<vm>.service` against the template above
  # and gets its full lifecycle config. (v0.1.3 fix — the previous
  # approach declared `systemd.services."nixling@${name}"` per VM,
  # which NixOS materialised as a separate file lacking ExecStart
  # → systemd refused with "Service has no ExecStart=, ExecStop=,
  # or SuccessAction=. Refusing.")
  systemd.targets.multi-user.wants =
    map (name: "nixling@${name}.service")
      (lib.attrNames (lib.filterAttrs (_: vm: vm.enable && vm.autostart) cfg.vms));
}
