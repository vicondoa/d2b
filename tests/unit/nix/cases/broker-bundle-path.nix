# nix-unit cases migrated from tests/broker-bundle-path-eval.sh.
#
# Asserts the three independent bundle-path declarations in the NixOS module
# tree all agree on the canonical /etc/nixling/bundle.json:
#
#   (A) nixos-modules/host-broker.nix `bundleManifestPath` fallback resolves
#       to /etc/nixling/bundle.json, and the broker ExecStart passes the
#       bundle to the binary via `--bundle-path`.
#   (B) nixos-modules/bundle.nix emits environment.etc."nixling/bundle.json"
#       (so the file lands at /etc/nixling/bundle.json) as the trusted
#       root:nixlingd 0640 artifact, plus the `nixlingBundleAcl` activation
#       hook that grants the lifecycle group READ-ONLY traverse/read ACLs
#       (never write) after etc/users.
#   (C) nixos-modules/host-daemon.nix daemonConfigJson artifacts.bundlePath
#       equals /etc/nixling/bundle.json, so daemon and broker share one path.
#
# (A) and (C) were source-text checks in the bash gate: evaluating
# serviceConfig.ExecStart forces the broker/daemon derivation build and
# recurses, so the bash inspected the .nix source directly. The faithful
# successor reads those sources via `builtins.readFile (flakeRoot + rel)` and
# matches per-line with `lib.hasInfix` (no IFD, no derivation build). (B)
# migrates to `mkEval` host introspection of `environment.etc` and
# `system.activationScripts`, mirroring the bash gate's nix eval.
{ mkEval, lib, flakeRoot, ... }:

let
  linesOf = rel: lib.splitString "\n" (builtins.readFile (flakeRoot + rel));
  brokerLines = linesOf "/nixos-modules/host-broker.nix";
  daemonLines = linesOf "/nixos-modules/host-daemon.nix";

  # Faithful `grep -F <needle>`: true iff some line contains the literal.
  has = lines: needle: {
    expr = lib.any (l: lib.hasInfix needle l) lines;
    expected = true;
  };

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
    nixling.daemonExperimental.enable = true;
  };

  cfg = (mkEval [ base ]).config;
  etc = cfg.environment.etc;
  bundleEtc = etc."nixling/bundle.json" or { };
  acl = cfg.system.activationScripts.nixlingBundleAcl or { };
  aclText = acl.text or "";
  aclDeps = acl.deps or [ ];
in
{
  # (A) host-broker.nix bundleManifestPath fallback + --bundle-path wiring.
  "broker-bundle-path/broker-default-fallback" =
    has brokerLines ''cfg.site.bundle.currentManifest or "/etc/nixling/bundle.json"'';
  "broker-bundle-path/exec-start-bundle-path-flag" =
    has brokerLines "--bundle-path";

  # (B) bundle.nix emits the trusted artifact at /etc/nixling/bundle.json.
  "broker-bundle-path/bundle-json-present" = {
    expr = builtins.hasAttr "nixling/bundle.json" etc;
    expected = true;
  };
  "broker-bundle-path/bundle-json-group" = {
    expr = bundleEtc.group or null;
    expected = "nixlingd";
  };
  "broker-bundle-path/bundle-json-mode" = {
    expr = bundleEtc.mode or null;
    expected = "0640";
  };

  # (B) nixlingBundleAcl: runs after etc/users, grants read-only traverse/read
  # ACLs to the lifecycle group, never write, never re-owns the artifact.
  "broker-bundle-path/acl-script-present" = {
    expr = cfg.system.activationScripts ? nixlingBundleAcl;
    expected = true;
  };
  "broker-bundle-path/acl-after-etc" = {
    expr = builtins.elem "etc" aclDeps;
    expected = true;
  };
  "broker-bundle-path/acl-after-users" = {
    expr = builtins.elem "users" aclDeps;
    expected = true;
  };
  "broker-bundle-path/acl-grants-directory" = {
    expr = lib.hasInfix "g:nixling:rx,m::rx" aclText;
    expected = true;
  };
  "broker-bundle-path/acl-grants-files" = {
    expr = lib.hasInfix "g:nixling:r,m::r" aclText;
    expected = true;
  };
  "broker-bundle-path/acl-no-write-grant" = {
    expr =
      !(lib.hasInfix "g:nixling:rw" aclText)
      && !(lib.hasInfix "g:nixling:rwx" aclText)
      && !(lib.hasInfix "m::rw" aclText)
      && !(lib.hasInfix "m::rwx" aclText);
    expected = true;
  };

  # (C) host-daemon.nix daemonConfigJson artifacts.bundlePath agreement.
  "broker-bundle-path/daemon-bundle-path" =
    has daemonLines ''bundlePath = "/etc/nixling/bundle.json"'';
}
