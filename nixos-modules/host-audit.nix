{ config, pkgs, lib, ... }:

{
  # ---------------------------------------------------------------------------
  # P6 W2 — §4.5 nixling-audit-check: daily oneshot + pre-rebuild hook gate.
  # Runs 'nixling audit --strict' daily and on demand. Operators with the
  # nixling-launcher polkit grant can trigger it via
  # 'systemctl start nixling-audit-check.service'.
  #
  # security-r8-audio-5: PATH must include openssh so the embedded
  # `sshd -T` call can resolve. Without it, the audit reads `null`
  # for ssh.host.PasswordAuthentication and STRICT-FAILs every
  # nixos-rebuild post-activation hook even though the host config
  # is correct. Same for jq / iproute2 / acl which the audit shells
  # out to from inside the wrappers — set a sufficient PATH so the
  # service environment mirrors what an interactive run sees.
  # ---------------------------------------------------------------------------
  systemd.services.nixling-audit-check = {
    description = "nixling security audit (strict mode)";
    documentation = [ "https://github.com/vicondoa/nixling/blob/main/docs/SECURITY.md" ];
    # Run once at boot (after network) so the first day doesn't need to
    # wait for the timer.
    wantedBy = [ "multi-user.target" ];
    after    = [ "network.target" ];
    path = with pkgs; [ openssh jq iproute2 acl nettools util-linux ];
    serviceConfig = {
      Type       = "oneshot";
      User       = "root";
      ExecStart  = "${config.nixling.cliBin} audit --strict";
      StandardOutput = "journal";
      StandardError  = "journal";
    };
  };

  systemd.timers.nixling-audit-check = {
    description = "Daily nixling security audit";
    wantedBy = [ "timers.target" ];
    timerConfig = {
      OnCalendar  = "daily";
      Persistent  = true;      # run missed runs after resume/boot
      RandomizedDelaySec = "10min";
      Unit        = "nixling-audit-check.service";
    };
  };
}
