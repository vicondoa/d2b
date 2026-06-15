# nix-unit cases migrated from tests/daemon-default-compat-eval.sh.
#
# Regression for the default of `nixling.daemonExperimental.enable`. In the
# daemon-only end state (ADR 0015) this is an obsolete compatibility option
# that simply defaults `true`: it is no longer evidence-auto-flipped by the
# `defaultSwitchReadiness` waves or by the per-wave evidence files under
# `defaultFlipEvidenceDir`. Explicit operator overrides still win in both
# directions (mkDefault/mkForce semantics preserved).
#
# Faithful migration note: the bash gate wrote per-wave evidence JSON files
# to disk and pointed `defaultFlipEvidenceDir` at them to exercise the
# historical evidence-gated default-flip. Because the default no longer
# depends on those files (reading `daemonExperimental.enable` never forces
# `validationEvidencePresent`), the five scenarios reproduce byte-identical
# boolean results WITHOUT any on-disk evidence — confirmed by probe before
# migration. The scenarios still set `defaultSwitchReadiness` /
# `defaultFlipEvidenceDir` exactly as the bash gate did so the cases mirror
# the original inputs; only the (now-inert) disk writes are dropped.
#
# Graphics-free, so contributes to the nix-unit check on every system.
{ mkEval, lib, ... }:

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

  # The wave set the historical flip gate iterated over (mirrors
  # `flipGateWaves` in nixos-modules/options-daemon.nix).
  flipGateWaves = [
    "w4Fu" "w5Fu" "w6Fu" "w7Fu" "w8Fu" "w9Fu" "p0" "p0Fu" "p1" "p2" "p3" "p4"
  ];

  allTrueReadiness = builtins.listToAttrs (map
    (w: { name = w; value = { implemented = true; validated = true; }; })
    flipGateWaves);

  # Same as allTrueReadiness but w9Fu.validated -> false.
  allTrueButOneValidated = builtins.listToAttrs (map
    (w: {
      name = w;
      value = { implemented = true; validated = w != "w9Fu"; };
    })
    flipGateWaves);

  enableOf = extra:
    (mkEval [ base extra ]).config.nixling.daemonExperimental.enable;
in
{
  # Scenario 1: every wave implemented + validated. Default stays true.
  "daemon-default-compat/gate-green-all" = {
    expr = enableOf ({ ... }: {
      nixling.daemonExperimental.defaultFlipEvidenceDir =
        "/var/lib/nixling/validated";
      nixling.defaultSwitchReadiness = allTrueReadiness;
    });
    expected = true;
  };

  # Scenario 2: one readiness entry not validated. Obsolete compat option
  # still defaults true.
  "daemon-default-compat/gate-red-readiness" = {
    expr = enableOf ({ ... }: {
      nixling.daemonExperimental.defaultFlipEvidenceDir =
        "/var/lib/nixling/validated";
      nixling.defaultSwitchReadiness = allTrueButOneValidated;
    });
    expected = true;
  };

  # Scenario 3: every readiness entry implemented + validated, but the
  # evidence directory is absent. Obsolete compat option still defaults
  # true (the default no longer reads evidence files).
  "daemon-default-compat/gate-red-evidence" = {
    expr = enableOf ({ ... }: {
      nixling.daemonExperimental.defaultFlipEvidenceDir =
        "/var/lib/nixling/validated-missing";
      nixling.defaultSwitchReadiness = allTrueReadiness;
    });
    expected = true;
  };

  # Scenario 4: gate fully green, operator pins enable = false. Operator
  # wins.
  "daemon-default-compat/override-false-wins-green" = {
    expr = enableOf ({ lib, ... }: {
      nixling.daemonExperimental.defaultFlipEvidenceDir =
        "/var/lib/nixling/validated";
      nixling.daemonExperimental.enable = lib.mkForce false;
      nixling.defaultSwitchReadiness = allTrueReadiness;
    });
    expected = false;
  };

  # Scenario 5: gate red (no validated flags, empty evidence dir), operator
  # opts in via mkForce true. Operator wins.
  "daemon-default-compat/override-true-wins-red" = {
    expr = enableOf ({ lib, ... }: {
      nixling.daemonExperimental.defaultFlipEvidenceDir =
        "/var/lib/nixling/validated-empty";
      nixling.daemonExperimental.enable = lib.mkForce true;
    });
    expected = true;
  };
}
