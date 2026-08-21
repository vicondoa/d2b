{ lib, modules, pkgs, ... }:

let
  module = builtins.head modules;
  evaluated = lib.evalModules {
    specialArgs = { inherit pkgs; };
    modules = [
      ({ lib, ... }: {
        options = {
          networking.hostName = lib.mkOption {
            type = lib.types.str;
            default = "provider-test";
          };
          microvm.hypervisor = lib.mkOption {
            type = lib.types.str;
            default = "";
          };
          microvm.cloud-hypervisor.extraArgs = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
          };
          security.tpm2.enable = lib.mkOption {
            type = lib.types.bool;
            default = false;
          };
          boot.kernelModules = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
          };
          environment.systemPackages = lib.mkOption {
            type = lib.types.listOf lib.types.package;
            default = [ ];
          };
          systemd.services = lib.mkOption {
            type = lib.types.attrsOf lib.types.anything;
            default = { };
          };
        };
      })
      module
    ];
  };
  config = evaluated.config;
in
{
  cases = {
    "provider-device-tpm/modules-evaluate" = {
      expr = builtins.deepSeq config.boot.kernelModules true;
      expected = true;
      propagateError = true;
    };

    "provider-device-tpm/tpm-crb-module" = {
      expr = {
        tpm = builtins.elem "tpm" config.boot.kernelModules;
        crb = builtins.elem "tpm_crb" config.boot.kernelModules;
        hypervisor = config.microvm.hypervisor;
        tpm2 = config.security.tpm2.enable;
      };
      expected = {
        tpm = true;
        crb = true;
        hypervisor = "cloud-hypervisor";
        tpm2 = true;
      };
    };
  };
}
