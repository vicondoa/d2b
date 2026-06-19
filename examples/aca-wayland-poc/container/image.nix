{
  # Build the nixling Wayland sandbox image (ADR 0032).
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
  # The productionized relay endpoint binary (nixling-relay), built from the
  # main workspace. Replaces the POC relay-bridge: the in-sandbox agent runs
  # `nixling-relay sender` authenticated by the sandbox managed identity
  # (Entra bearer, plane 2) — no SAS key enters the workload.
  nixlingRelaySrc = builtins.path {
    name = "nixling-packages-src";
    path = ../../../packages;
    filter =
      path: _type:
      let
        base = builtins.baseNameOf path;
      in
      base != "target" && base != ".cargo";
  };
  nixlingRelay = pkgs.rustPlatform.buildRustPackage {
    pname = "nixling-relay";
    version = "0.0.0-bootstrap";
    src = nixlingRelaySrc;
    cargoLock = {
      lockFile = ../../../packages/Cargo.lock;
      outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
    };
    cargoBuildFlags = [
      "-p"
      "nixling-provider-relay"
      "--bin"
      "nixling-relay"
    ];
    env.CARGO_BUILD_RUSTC_WRAPPER = "";
    doCheck = false;
  };

  # The in-sandbox agent: fetches the MI Entra token, runs `nixling-relay
  # sender`, then `waypipe server` + the app (see bridge/nixling-sandbox-agent.sh).
  agent = pkgs.writeShellApplication {
    name = "nixling-sandbox-agent";
    runtimeInputs = [
      pkgs.waypipe
      pkgs.foot
      pkgs.coreutils
      nixlingRelay
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
    nixlingRelay
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
