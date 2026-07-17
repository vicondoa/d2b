{ system ? builtins.currentSystem
, pkgs ? import <nixpkgs> { inherit system; }
}:

let
  inherit (pkgs) lib;
  identity = import ../../../nixos-modules/v2-identity.nix;
  realmId = identity.deriveRealmId "work.local-root";
  workloadId = identity.deriveWorkloadId realmId "desktop";
  swtpmRoleId = identity.deriveRoleId realmId workloadId "swtpm";
  expectedSocket =
    "/run/d2b/r/${realmId}/w/${workloadId}/roles/${swtpmRoleId}/tpm.sock";

  resources = lib.evalModules {
    modules = [
      ../../../nixos-modules/options-realms.nix
      ../../../nixos-modules/index.nix
      ../../../nixos-modules/realm-device-rows.nix
      ({ lib, ... }: {
        options.assertions = lib.mkOption {
          type = lib.types.listOf lib.types.attrs;
          default = [ ];
        };
        config.d2b.realms.work = {
          path = "work.local-root";
          providers.runtime = {
            type = "runtime";
            implementationId = "cloud-hypervisor";
          };
          providers.devices = {
            type = "device";
            implementationId = "host-mediated";
          };
          workloads.desktop = {
            providerRefs = {
              runtime = "runtime";
              device = "devices";
            };
            tpm.enable = true;
          };
        };
      })
    ];
  };
  tpmRow = builtins.head resources.config.d2b._index.devices.list;
  tpmLease = builtins.head
    resources.config.d2b._index.devices.allocatorLeaseRequests;

  guest = import (pkgs.path + "/nixos/lib/eval-config.nix") {
    inherit system;
    specialArgs = {
      d2bRealmId = realmId;
      d2bWorkloadId = workloadId;
      d2bRoleIds.swtpm = swtpmRoleId;
    };
    modules = [
      ../../../nixos-modules/components/tpm.nix
      ({ lib, ... }: {
        options.microvm = lib.mkOption {
          type = lib.types.attrs;
          default = { };
        };
        config = {
          boot.loader.grub.enable = false;
          boot.loader.systemd-boot.enable = false;
          boot.initrd.includeDefaultModules = false;
          fileSystems."/" = {
            device = "tmpfs";
            fsType = "tmpfs";
          };
          system.stateVersion = "25.11";
        };
      })
    ];
  };

  extraArgs = guest.config.microvm.cloud-hypervisor.extraArgs;
  checks = [
    (if extraArgs == [ "--tpm" "socket=${expectedSocket}" ] then null else
      throw "smoke-eval-tpm: cloud-hypervisor TPM socket is not canonical")
    (if builtins.hasAttr "tpm2-flush-sessions" guest.config.systemd.services
      then null else throw "smoke-eval-tpm: stale-session flush service is missing")
    (if builtins.hasAttr "tpm2-srk-provision" guest.config.systemd.services
      then null else throw "smoke-eval-tpm: SRK provisioning service is missing")
    (if tpmRow.roleId == swtpmRoleId && tpmRow.endpointPath == expectedSocket
      then null else throw "smoke-eval-tpm: TPM resource row is not canonical")
    (if tpmLease.resourceId == "device-tpm-${workloadId}"
      && tpmLease.share == "exclusive"
      then null else throw "smoke-eval-tpm: TPM allocator lease is not workload-exclusive")
  ];
in
builtins.deepSeq checks
  (pkgs.runCommand "d2b-smoke-eval-realm-tpm" { } "touch $out")
