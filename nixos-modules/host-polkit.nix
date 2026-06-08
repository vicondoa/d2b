{ lib, ... }:

let
  # ---------------------------------------------------------------------------
  # P6 ph6-p6-polkit-retire — daemon-only end-state.
  #
  # Pre-P6 this module generated an exact-unit allowlist covering every
  # per-VM sidecar the bash CLI drove via `systemctl <verb>
  # <unit>` (the W2-followup C1 grant): `nixling@<vm>.service`,
  # `nixling-<vm>-{gpu,snd,swtpm,store-sync}.service`,
  # `nixling-sys-<env>-usbipd-{proxy,backend}.{service,socket}`, plus a
  # second rule scoped to the per-VM `nixling-<vm>-gpu` system user
  # granting it start/stop/restart of its paired
  # `nixling-<vm>-snd.service`.
  #
  # All of those grants are vestigial post-clean-break (ADR 0015):
  #
  #   * `ph6-p6-cli-nix-migrations` + `ph6-remove-systemd-emission`
  #     delete the bash CLI and every per-VM systemd template the
  #     allowlist named — there is no longer any unit shaped like
  #     `nixling@<vm>` / `nixling-<vm>-*` / `nixling-sys-<env>-*`
  #     for polkit to be asked about.
  #   * The W14c bash fallback bridge was retired in P4; mutating verbs
  #     run daemon-only end-to-end. The operator-facing control plane
  #     is the daemon's public socket (group-readable to `nixlingd`),
  #     authorised at accept time via SO_PEERCRED — polkit is no
  #     longer in the per-VM lifecycle path.
  #
  # What is KEPT: the launcher-group allowlist for the two daemon-only
  # singleton units that operators still drive directly with
  # `systemctl`:
  #
  #   * `nixlingd.service` — the public daemon. Operators occasionally
  #     restart it after a `nixos-rebuild switch` (it carries
  #     `restartIfChanged=false` per the daemon-lifecycle invariant).
  #   * `nixling-priv-broker.service` + `nixling-priv-broker.socket` —
  #     the privileged broker pair. Socket-activated; operators may
  #     bounce them to recover from a stuck handler.
  #
  # Verbs are limited to `start`, `stop`, `restart`. `reload`,
  # `try-restart`, `enable`, `disable`, `mask`, `manage-unit-files`,
  # and `reload-daemon` still require the polkit default
  # (admin-password) path. Action ids other than
  # `org.freedesktop.systemd1.manage-units` are not granted.
  #
  # Default-deny invariant: the allowlist is a literal three-element
  # array; an unknown unit name falls through the for-loop and the
  # rule returns `undefined`, which polkit treats as "this rule has
  # no opinion" — control passes to the next rule, ultimately to the
  # password-prompt default.
  # ---------------------------------------------------------------------------
  launcherAllowedUnits = [
    "nixlingd.service"
    "nixling-priv-broker.service"
    "nixling-priv-broker.socket"
  ];

  launcherAllowedUnitsJs =
    "[" + (lib.concatMapStringsSep ", " (u: ''"${u}"'') launcherAllowedUnits) + "]";
in
{
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
  '';
}
