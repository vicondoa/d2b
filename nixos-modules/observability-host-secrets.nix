# Host-side secret provisioning for the observability stack.
#
# v0.2.0 originally generated the observability UI credentials inside
# the stack VM via `system.activationScripts`. That put secrets in the
# wrong place: anything on the HOST that needs them (a launcher, an
# external health probe, a backup of stack credentials) had to cross the
# VM boundary to read them. The trust flow was pointing the wrong way.
#
# This module fixes that by generating both secrets on the HOST and
# sharing them into the stack VM read-only over virtiofs. The flow
# is identical in structure to the per-VM `host-keys/` share
# managed by `host-keys.nix` + `store.nix`:
#
#   Host:    `<stateDir>/observability/*` (root:root 0444 under a
#            root:root 0700 parent)
#   Share:   virtiofs tag `nl-obs-sec` → stack-VM mount
#            `/run/nixling-obs-secrets/` (read-only)
#   Guest:   stack services' `LoadCredential`
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
# Active only when `nixling.observability.enable = true`.
{ config, pkgs, lib, ... }:

let
  cfg = config.nixling;
  obsCfg = cfg.observability;

  hostSecretsDir = "${cfg.site.stateDir}/observability";
  hostSecretKeyPath = "${hostSecretsDir}/grafana-secret-key";
  hostAdminPasswordPath = "${hostSecretsDir}/grafana-admin-password";
  hostSignozJwtSecretPath = "${hostSecretsDir}/signoz-jwt-secret";
  hostSignozRootPasswordPath = "${hostSecretsDir}/signoz-root-password";
  hostClickHousePasswordPath = "${hostSecretsDir}/clickhouse-password";

  guestSecretsMountPoint = "/run/nixling-obs-secrets";

  manageSecretKey = obsCfg.grafana.secretKeyFile == null;
  manageAdminPassword = obsCfg.grafana.adminPasswordFile == null;
  signozSecretSpecs = [
    {
      name = "SignozJwtSecret";
      path = hostSignozJwtSecretPath;
      source = obsCfg.signoz.jwtSecretFile;
      minSize = 32;
      bytes = 64;
    }
    {
      name = "SignozRootPassword";
      path = hostSignozRootPasswordPath;
      source = obsCfg.signoz.rootPasswordFile;
      minSize = 16;
      bytes = 48;
    }
    {
      name = "ClickHousePassword";
      path = hostClickHousePasswordPath;
      source = obsCfg.signoz.clickhousePasswordFile;
      minSize = 16;
      bytes = 48;
    }
  ];
in
{
  config = lib.mkIf obsCfg.enable (lib.mkMerge [
    # Host-side activation: idempotently generate both secrets at
    # activation. Same pattern as `host-keys.nix`: atomic install via
    # tempfile + `mv -f`, repair perms on every activation.
    {
      system.activationScripts = lib.mkMerge [
        (lib.mkIf (manageSecretKey || manageAdminPassword || signozSecretSpecs != [ ]) {
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
        (builtins.listToAttrs (map
          (spec: {
            name = "nixlingObservabilityHost${spec.name}";
            value = lib.stringAfter [ "nixlingObservabilityHostSecretsDir" ] (
              if spec.source != null then ''
                ${pkgs.coreutils}/bin/install -m 0444 -o root -g root \
                  ${lib.escapeShellArg (toString spec.source)} \
                  ${lib.escapeShellArg spec.path}
              '' else ''
              file=${lib.escapeShellArg spec.path}
              if [ -s "$file" ] && [ "$(${pkgs.coreutils}/bin/stat -c %s "$file")" -ge ${toString spec.minSize} ]; then
                :
              else
                umask 077
                tmp="$file.tmp.$$"
                ${pkgs.coreutils}/bin/rm -f "$tmp"
                ${pkgs.coreutils}/bin/head -c ${toString spec.bytes} /dev/urandom \
                  | ${pkgs.coreutils}/bin/base64 > "$tmp"
                ${pkgs.coreutils}/bin/chmod 0444 "$tmp"
                ${pkgs.coreutils}/bin/chown root:root "$tmp"
                ${pkgs.coreutils}/bin/mv -f "$tmp" "$file"
              fi
              ${pkgs.coreutils}/bin/chmod 0444 "$file"
              ${pkgs.coreutils}/bin/chown root:root "$file"
            ''
            );
          })
          signozSecretSpecs))
      ];
    }

    {
      system.activationScripts.nixlingObservabilityHostSecretShareAcls =
        lib.stringAfter [ "nixlingObservabilityHostSecretsDir" ] ''
          set -u
          processes=/etc/nixling/processes.json
          if [ -r "$processes" ]; then
            obs_uid="$(${pkgs.jq}/bin/jq -r --arg vm ${lib.escapeShellArg obsCfg.vmName} '.vms[] | select(.vm == $vm) | .nodes[] | select(.id == "virtiofsd-nl-obs-sec") | .profile.uid' "$processes" 2>/dev/null | ${pkgs.coreutils}/bin/head -n1)"
            case "$obs_uid" in
              ""|null) ;;
              *[!0-9]*) ;;
              *)
                ${pkgs.acl}/bin/setfacl -m "u:$obs_uid:--x" ${lib.escapeShellArg cfg.site.stateDir} 2>/dev/null || true
                ${pkgs.acl}/bin/setfacl -m "u:$obs_uid:r-x" ${lib.escapeShellArg hostSecretsDir} 2>/dev/null || true
                for secret in ${lib.escapeShellArg hostSecretsDir}/*; do
                  [ -f "$secret" ] || continue
                  ${pkgs.acl}/bin/setfacl -m "u:$obs_uid:r--" "$secret" 2>/dev/null || true
                done
                ;;
            esac
          fi
        '';
    }

    # NOTE: the matching virtiofs share into sys-obs is
    # declared in `nixos-modules/host.nix`'s composedConfig pass
    # (v1.1 moved it out of store.nix to avoid module-system infinite
    # recursion). The share lives inside the per-VM
    # `microvm.shares` list at
    # `config.nixling._computed.sys-obs.config.microvm.shares`.
    # Adding the share from here would lose to the mkForce in
    # host.nix. Coordinating via host.nix also lines up the obs
    # secrets share with the other framework-managed shares
    # (`ro-store`, `nl-meta`, `nl-hkeys`).
  ]);
}
