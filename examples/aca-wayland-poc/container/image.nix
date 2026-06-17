{
  # Build the nixling Wayland sandbox image (ADR 0032, Wave P0).
  #
  #   nix-build image.nix            # -> ./result (an OCI image tar.gz)
  #   ./build-and-push.sh            # build + push to the deployed ACR
  #
  # waypipe and foot are taken from the same nixpkgs as the host so the
  # in-sandbox `waypipe server` is byte-identical to the host
  # `waypipe client` (waypipe requires matching versions on both ends).
  pkgs ? import <nixpkgs> { system = "x86_64-linux"; },
}:
let
  # A monospace font + fontconfig so `foot` can render.
  fontsConf = pkgs.makeFontsConf {
    fontDirectories = [ pkgs.dejavu_fonts ];
  };

  # Global foot config: pins DejaVu Sans Mono and silences the
  # non-monospace fallback-font warning.
  footConfig = pkgs.writeTextDir "etc/xdg/foot/foot.ini" ''
    font=DejaVu Sans Mono:size=11
    [tweak]
    font-monospace-warn=no
  '';

  # The relay bridge (sender side): tunnels the waypipe stream out over
  # the Azure Relay hybrid connection. Built from the same nixpkgs so it
  # links the image's glibc and runs inside the sandbox. The source is
  # filtered to exclude the cargo `target/` dir.
  relayBridgeSrc = builtins.path {
    name = "nixling-relay-bridge-src";
    path = ../relay-bridge;
    filter = path: _type: builtins.baseNameOf path != "target";
  };
  relayBridge = pkgs.rustPlatform.buildRustPackage {
    pname = "nixling-relay-bridge";
    version = "0.0.0-poc";
    src = relayBridgeSrc;
    cargoLock.lockFile = ../relay-bridge/Cargo.lock;
    doCheck = false;
  };

  # The in-sandbox agent: tunnels `waypipe server` + the app out over the
  # Azure Relay hybrid connection (see bridge/nixling-sandbox-agent.sh).
  agent = pkgs.writeShellApplication {
    name = "nixling-sandbox-agent";
    runtimeInputs = [
      pkgs.waypipe
      pkgs.foot
      pkgs.coreutils
      relayBridge
    ];
    text = builtins.readFile ./bridge/nixling-sandbox-agent.sh;
  };
in
pkgs.dockerTools.buildLayeredImage {
  name = "nixling-wayland";
  tag = "latest";

  contents = [
    pkgs.waypipe
    pkgs.foot
    pkgs.bashInteractive
    pkgs.coreutils
    pkgs.dejavu_fonts
    pkgs.fontconfig
    pkgs.cacert
    agent
    relayBridge
    footConfig
    pkgs.dockerTools.fakeNss # minimal /etc/passwd + /etc/group + nobody
  ];

  # World-writable /tmp + the runtime dir the agent uses.
  extraCommands = ''
    mkdir -p tmp run/nixling
    chmod 1777 tmp
    chmod 0700 run/nixling
  '';

  config = {
    Entrypoint = [ "${agent}/bin/nixling-sandbox-agent" ];
    Env = [
      "XDG_RUNTIME_DIR=/run/nixling"
      "XDG_CONFIG_DIRS=/etc/xdg"
      "FONTCONFIG_FILE=${fontsConf}"
      "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
      "LC_ALL=C.UTF-8"
      "NIXLING_APP=foot"
      "NIXLING_WP_COMPRESS=zstd"
      "TERM=xterm-256color"
    ];
    Labels = {
      "org.nixling.component" = "aca-wayland-poc";
      "org.nixling.adr" = "0032";
    };
  };
}
