# Host-side secret provisioning for the observability stack.
#
# v0.2.0 originally generated the Grafana `admin_password` and
# `secret_key` inside the stack VM via `system.activationScripts`.
# That put two secrets in the wrong place: anything on the HOST that
# needs them (a Grafana launcher, an external health probe, a backup
# of the stack credentials) had to cross the VM boundary to read
# them — which in practice forced consumers to add a SSH-able
# operator user + sudoers rule inside `sys-obs-stack` just to claw
# the password back out. The trust flow was pointing the wrong way.
#
# This module fixes that by generating both secrets on the HOST and
# sharing them into the stack VM read-only over virtiofs. The flow
# is identical in structure to the per-VM `host-keys/` share
# managed by `host-keys.nix` + `store.nix`:
#
#   Host:    `<stateDir>/observability/{grafana-secret-key,
#             grafana-admin-password}` (root:root 0400)
#   Share:   virtiofs tag `nl-obs-sec` → stack-VM mount
#            `/run/nixling-obs-secrets/` (read-only)
#   Guest:   `systemd.services.grafana.serviceConfig.LoadCredential`
#            points at the in-VM mount path (see
#            `components/observability/stack.nix`).
#
# Consumers no longer need to declare anything inside the stack VM
# to gain host-side access to the admin password: `sudo cat
# <stateDir>/observability/grafana-admin-password` from the host is
# the supported path. A focused sudoers rule on the host (in
# `/etc/nixos`) is the consumer's responsibility; the framework
# does not assume one operator name.
#
# Active only when `nixling.observability.enable = true`. Honours
# the existing `nixling.observability.grafana.{secretKeyFile,
# adminPasswordFile}` overrides — if either is set, this module
# leaves that secret alone (sops-nix / agenix users keep working).
{ config, pkgs, lib, ... }:

let
  cfg = config.nixling;
  obsCfg = cfg.observability;

  hostSecretsDir = "${cfg.site.stateDir}/observability";
  hostSecretKeyPath = "${hostSecretsDir}/grafana-secret-key";
  hostAdminPasswordPath = "${hostSecretsDir}/grafana-admin-password";

  guestSecretsMountPoint = "/run/nixling-obs-secrets";

  manageSecretKey = obsCfg.grafana.secretKeyFile == null;
  manageAdminPassword = obsCfg.grafana.adminPasswordFile == null;
in
{
  config = lib.mkIf obsCfg.enable (lib.mkMerge [
    # Host-side activation: idempotently generate both secrets at
    # activation. Same pattern as `host-keys.nix`: atomic install via
    # tempfile + `mv -f`, repair perms on every activation.
    {
      system.activationScripts = lib.mkMerge [
        (lib.mkIf (manageSecretKey || manageAdminPassword) {
          nixlingObservabilityHostSecretsDir = lib.stringAfter [ "users" ] ''
            ${pkgs.coreutils}/bin/install -d -m 0700 -o root -g root \
              ${lib.escapeShellArg hostSecretsDir}
          '';
        })
        (lib.mkIf manageSecretKey {
          nixlingObservabilityHostSecretKey = lib.stringAfter
            [ "nixlingObservabilityHostSecretsDir" ] ''
            file=${lib.escapeShellArg hostSecretKeyPath}
            if [ -s "$file" ] && [ "$(${pkgs.coreutils}/bin/stat -c %s "$file")" -ge 32 ]; then
              :
            else
              umask 077
              tmp="$file.tmp.$$"
              ${pkgs.coreutils}/bin/rm -f "$tmp"
              ${pkgs.coreutils}/bin/head -c 64 /dev/urandom \
                | ${pkgs.coreutils}/bin/base64 > "$tmp"
              ${pkgs.coreutils}/bin/chmod 0400 "$tmp"
              ${pkgs.coreutils}/bin/chown root:root "$tmp"
              ${pkgs.coreutils}/bin/mv -f "$tmp" "$file"
            fi
            ${pkgs.coreutils}/bin/chmod 0400 "$file"
            ${pkgs.coreutils}/bin/chown root:root "$file"
          '';
        })
        (lib.mkIf manageAdminPassword {
          nixlingObservabilityHostAdminPassword = lib.stringAfter
            [ "nixlingObservabilityHostSecretsDir" ] ''
            file=${lib.escapeShellArg hostAdminPasswordPath}
            if [ -s "$file" ]; then
              :
            else
              umask 077
              tmp="$file.tmp.$$"
              ${pkgs.coreutils}/bin/rm -f "$tmp"
              ${pkgs.coreutils}/bin/head -c 48 /dev/urandom \
                | ${pkgs.coreutils}/bin/base64 > "$tmp"
              ${pkgs.coreutils}/bin/chmod 0400 "$tmp"
              ${pkgs.coreutils}/bin/chown root:root "$tmp"
              ${pkgs.coreutils}/bin/mv -f "$tmp" "$file"
            fi
            ${pkgs.coreutils}/bin/chmod 0400 "$file"
            ${pkgs.coreutils}/bin/chown root:root "$file"
          '';
        })
      ];
    }

    # NOTE: the matching virtiofs share into sys-obs-stack is
    # declared in `nixos-modules/host.nix`'s composedConfig pass
    # (v1.1-final moved it out of store.nix to avoid module-system
    # infinite recursion). The share lives inside the per-VM
    # `microvm.shares` list at
    # `config.nixling._computed.sys-obs-stack.config.microvm.shares`.
    # Adding the share from here would lose to the mkForce in
    # host.nix. Coordinating via host.nix also lines up the obs
    # secrets share with the other framework-managed shares
    # (`ro-store`, `nl-meta`, `nl-hkeys`).
  ]);
}
