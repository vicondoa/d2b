# nix-unit cases migrated from tests/broker-socket-activation-eval.sh.
#
# Asserts host-broker.nix wires nixling-priv-broker for socket activation:
#
#   (A) ExecStart does NOT pass --socket-path (with SD_LISTEN_FDS the broker
#       MUST adopt the inherited fd, not self-bind the path).
#   (B) systemd.sockets.nixling-priv-broker exists (socket-activated).
#   (C) socketConfig.FileDescriptorName is "priv.sock" (matched by
#       adopt_listen_fd() against LISTEN_FDNAMES).
#   (D) socketConfig.ListenSequentialPacket is /run/nixling/priv.sock.
#
# (A) was a source-text check in the bash gate: evaluating
# serviceConfig.ExecStart forces the broker derivation build and recurses,
# so the source is inspected directly. The faithful successor reads
# host-broker.nix via `builtins.readFile (flakeRoot + rel)` and asserts no
# NON-COMMENT line carries --socket-path (the source mentions the flag only
# in an explanatory comment, which is excluded), plus that the ExecStart
# assignment is still present (file not hollowed out). (B)-(D) migrate to
# `mkEval` introspection of `systemd.sockets`, mirroring the bash gate's
# nix eval — socketConfig fields are safe to force (unlike ExecStart).
{ mkEval, lib, flakeRoot, ... }:

let
  brokerLines = lib.splitString "\n"
    (builtins.readFile (flakeRoot + "/nixos-modules/host-broker.nix"));

  # A source line is a comment iff its first non-whitespace char is '#'.
  isComment = l: builtins.match "[[:space:]]*#.*" l != null;
  codeLines = lib.filter (l: !(isComment l)) brokerLines;

  # The bash gate used the MINIMAL daemon-only config (no site/envs) for the
  # socket eval; the socket unit shape is independent of site/envs. Mirror it.
  minimal = { ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    nixling.daemonExperimental.enable = true;
  };

  cfg = (mkEval [ minimal ]).config;
  sockCfg = cfg.systemd.sockets.nixling-priv-broker.socketConfig or { };
in
{
  # (A) No uncommented --socket-path in host-broker.nix.
  "broker-socket-activation/no-socket-path-flag" = {
    expr = lib.any (l: lib.hasInfix "--socket-path" l) codeLines;
    expected = false;
  };
  # (A) ExecStart assignment is present (sanity: file not hollowed out).
  "broker-socket-activation/exec-start-present" = {
    expr = lib.any (l: lib.hasInfix "ExecStart =" l) brokerLines;
    expected = true;
  };

  # (B) Socket unit exists (broker is socket-activated).
  "broker-socket-activation/has-socket" = {
    expr = cfg.systemd.sockets ? nixling-priv-broker;
    expected = true;
  };
  # (C) FileDescriptorName matches the name adopt_listen_fd() validates.
  "broker-socket-activation/fd-name" = {
    expr = sockCfg.FileDescriptorName or "";
    expected = "priv.sock";
  };
  # (D) Socket listens at the canonical private socket path.
  "broker-socket-activation/listen-seq-packet" = {
    expr = sockCfg.ListenSequentialPacket or "";
    expected = "/run/nixling/priv.sock";
  };

  "broker-socket-activation/socket-after-tmpfiles" = {
    expr = builtins.elem "systemd-tmpfiles-setup.service"
      (cfg.systemd.sockets.nixling-priv-broker.after or [ ]);
    expected = true;
  };

  "broker-socket-activation/socket-requires-tmpfiles" = {
    expr = builtins.elem "systemd-tmpfiles-setup.service"
      (cfg.systemd.sockets.nixling-priv-broker.requires or [ ]);
    expected = true;
  };
}
