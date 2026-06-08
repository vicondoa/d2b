{ config, lib, ... }:

let
  cfg = config.nixling;

  # ---------------------------------------------------------------------------
  # W2-followup C1: exact-unit allowlist for the nixling-launcher polkit
  # grant. Generated at NixOS eval time from `config.nixling.{vms,envs}`
  # so the rule covers exactly the units the framework owns — no
  # prefix wildcards, no `microvm@*` blanket grant.
  #
  # Each per-VM entry includes only the sidecar units that are actually
  # materialised for that VM (graphics → -gpu, audio → -snd, tpm →
  # -swtpm; -store-sync exists for every declared VM via store.nix;
  # `nixling@<vm>` exists for every declared VM via the wrapper
  # template). Per-env entries cover the usbipd
  # backend/proxy/proxy-socket triplet.
  #
  # The companion JS rule below (`nixling-<vm>-gpu → -snd` fallback)
  # is unchanged in scope — still exactly the paired snd sidecar for
  # the matching VM — but now also enforces the verb allowlist.
  # ---------------------------------------------------------------------------
  launcherAllowedUnits =
    let
      enabledVms = lib.filterAttrs (_: vm: vm.enable) cfg.vms;

      # Per-VM sidecar units. nixling@<vm>.service is the wrapper
      # users actually drive; ExecStop on the wrapper propagates the
      # stop to microvm@<vm> so there is no need to grant
      # microvm@<vm> separately. microvm-virtiofsd@<vm> is restarted
      # by the CLI from inside a sudo-A heredoc (root); not a
      # polkit-grant code path.
      perVmUnits = name: vm:
        [ "nixling@${name}.service"
          "nixling-${name}-store-sync.service"
        ]
        ++ lib.optional vm.graphics.enable "nixling-${name}-gpu.service"
        ++ lib.optional vm.audio.enable    "nixling-${name}-snd.service"
        ++ lib.optional vm.tpm.enable      "nixling-${name}-swtpm.service";

      # Per-env system units (usbipd backend + proxy + proxy socket).
      # Materialised by network.nix for every declared env.
      perEnvUnits = envName: _:
        [ "nixling-sys-${envName}-usbipd-proxy.service"
          "nixling-sys-${envName}-usbipd-proxy.socket"
          "nixling-sys-${envName}-usbipd-backend.service"
        ];

      # Host-side singletons.
      systemUnits = [
        "nixling-audit-check.service"
      ];
    in
    lib.unique (lib.sort lib.lessThan (lib.flatten (
      [ systemUnits ]
      ++ lib.mapAttrsToList perVmUnits enabledVms
      ++ lib.mapAttrsToList perEnvUnits cfg.envs
    )));

  # JS array literal of the allowlist, embedded into the polkit rule
  # below. Empty allowlist → an `[]` array → the inclusion check is
  # `false` for every unit, so the launcher grants nothing
  # (default-deny invariant preserved).
  launcherAllowedUnitsJs =
    "[" + (lib.concatMapStringsSep ", " (u: ''"${u}"'') launcherAllowedUnits) + "]";
in
{
  # ---------------------------------------------------------------------------
  # W2-followup C1: nixling-launcher polkit grant — exact-unit allowlist.
  #
  # Members of `nixling-launcher` may run start/stop/restart against
  # exactly the units this framework owns, generated above into
  # `launcherAllowedUnits` at NixOS eval time. Anything else falls
  # through to the polkit default (deny / password prompt).
  #
  # Verb allowlist is enforced via `action.lookup("verb")`. Only
  # `start`, `stop`, `restart` are granted — `reload`,
  # `try-restart`, `enable`, `disable`, `mask`, etc. still require
  # password. Other action ids (manage-unit-files, reload-daemon,
  # manage-units for non-listed units) are not granted.
  #
  # Replaces the W2-era prefix match (`nixling@*`, `microvm@*`,
  # `microvm-virtiofsd@*`, `nixling-*`) which was overbroad: the
  # bare `microvm@*` wildcard granted control over any microvm.nix-
  # declared VM in the system, not just the ones nixling owns, and
  # the open verb set let `nixling-launcher` members mask units or
  # send them reload signals.
  #
  # security-r8-audio-3 (kept): the companion rule below allows the
  # per-VM `nixling-<vm>-gpu` system user (NOT a member of
  # nixling-launcher) to start/stop/restart ONLY its paired
  # `nixling-<vm>-snd.service`. This is the fallback used by
  # microvm-run's audioArgsScript when CH is launched directly. The
  # verb allowlist applies here too.
  # ---------------------------------------------------------------------------
  security.polkit.extraConfig = ''
    polkit.addRule(function(action, subject) {
      if (action.id !== "org.freedesktop.systemd1.manage-units") return;
      if (!subject.isInGroup("nixling-launcher")) return;
      var verb = action.lookup("verb") || "";
      if (verb !== "start" && verb !== "stop" && verb !== "restart") return;
      var unit = action.lookup("unit") || "";
      var allowed = ${launcherAllowedUnitsJs};
      for (var i = 0; i < allowed.length; i++) {
        if (unit === allowed[i]) {
          return polkit.Result.YES;
        }
      }
    });
    polkit.addRule(function(action, subject) {
      if (action.id !== "org.freedesktop.systemd1.manage-units") return;
      var verb = action.lookup("verb") || "";
      if (verb !== "start" && verb !== "stop" && verb !== "restart") return;
      var user = subject.user || "";
      // User name shape: nixling-<vm>-gpu. Strip prefix + suffix to
      // recover the VM name, then allow only that VM's snd sidecar.
      var prefix = "nixling-";
      var suffix = "-gpu";
      if (user.indexOf(prefix) !== 0) return;
      if (user.length <= prefix.length + suffix.length) return;
      if (user.substring(user.length - suffix.length) !== suffix) return;
      var vm = user.substring(prefix.length, user.length - suffix.length);
      if (!vm) return;
      var unit = action.lookup("unit") || "";
      if (unit === ("nixling-" + vm + "-snd.service")) {
        return polkit.Result.YES;
      }
    });
  '';
}
