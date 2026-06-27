# Guest-side auditd support for d2b VMs. Imported by host.nix
# whenever a VM sets `d2b.vms.<name>.audit.enable = true`.
{ config, lib, ... }:

let
  cfg = config.d2b.audit;
in
{
  options.d2b.audit = {
    enable = lib.mkEnableOption "guest-side auditd with forwarding to the observability pipeline";

    rules = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [
        "-w /etc/passwd -p wa -k identity"
        "-w /etc/shadow -p wa -k identity"
        "-w /etc/sudoers -p wa -k priv-esc"
      ];
      description = ''
        Curated audit rules for guest-side auditd. The default excludes
        `execve` argv capture because command lines routinely carry
        secrets; add that rule explicitly only for short-lived,
        high-sensitivity audits.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    security.auditd = {
      enable = true;
      plugins.syslog.active = true;
    };
    security.audit = {
      enable = true;
      rules = cfg.rules;
    };
  };
}
