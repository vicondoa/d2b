{
  # Build the d2b Wayland sandbox image (ADR 0032).
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

  # The handshake-gated relay endpoint binary, built from the main workspace.
  # The gateway-generated in-sandbox command runs `d2b-gateway-relay
  # sender` with relay auth supplied by the gateway (P0: a short-lived Send
  # bearer, never the rule key) and sends the d2b per-session display
  # credential as the relay prologue before any Waypipe byte flows.
  d2bRelaySrc = builtins.path {
    name = "d2b-packages-src";
    path = ../../../packages;
    filter =
      path: _type:
      let
        base = builtins.baseNameOf path;
      in
      base != "target" && base != ".cargo";
  };
  d2bGatewayRelay = pkgs.rustPlatform.buildRustPackage {
    pname = "d2b-gateway-relay";
    version = "0.0.0-bootstrap";
    src = d2bRelaySrc;
    cargoLock = {
      lockFile = ../../../packages/Cargo.lock;
      outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
    };
    cargoBuildFlags = [
      "-p"
      "d2b-gateway-runtime"
      "--bin"
      "d2b-gateway-relay"
    ];
    env.CARGO_BUILD_RUSTC_WRAPPER = "";
    doCheck = false;
  };

  msiTokenHelper = pkgs.writeShellApplication {
    name = "d2b-msi-token";
    runtimeInputs = [
      pkgs.bashInteractive
      pkgs.coreutils
    ];
    text = ''
      set -euo pipefail
      resource="''${1:?usage: d2b-msi-token <resource> [client-id]}"
      client_id="''${2:-''${D2B_MI_CLIENT_ID:-}}"
      ep="''${IDENTITY_ENDPOINT:?IDENTITY_ENDPOINT not injected}"
      rest="''${ep#http://}"
      hostport="''${rest%%/*}"
      path="/''${rest#*/}"
      host="''${hostport%%:*}"
      port="''${hostport##*:}"
      [ "$host" = "$port" ] && port=80
      resource_enc="''${resource//:/%3A}"
      resource_enc="''${resource_enc//\\//%2F}"
      q="?api-version=2019-08-01&resource=$resource_enc"
      if [ -n "$client_id" ]; then
        q="$q&client_id=$client_id"
      fi
      exec 3<>"/dev/tcp/$host/$port"
      printf 'GET %s%s HTTP/1.1\r\nHost: %s\r\nX-IDENTITY-HEADER: %s\r\nMetadata: true\r\nConnection: close\r\n\r\n' \
        "$path" "$q" "$host" "''${IDENTITY_HEADER:-}" >&3
      out="$(cat <&3)"
      exec 3>&-
      out="''${out#*\"access_token\":\"}"
      token="''${out%%\"*}"
      [ -n "$token" ] && [ "$token" != "$out" ]
      printf '%s\n' "$token"
    '';
  };

  # The in-sandbox legacy entrypoint remains available for manual probes; the
  # gateway-generated command uses `d2b-gateway-relay` and `d2b-msi-token`
  # directly.
  agent = pkgs.writeShellApplication {
    name = "d2b-sandbox-agent";
    runtimeInputs = [
      pkgs.waypipe
      pkgs.foot
      pkgs.coreutils
      d2bGatewayRelay
      msiTokenHelper
    ];
    text = builtins.readFile ./bridge/d2b-sandbox-agent.sh;
  };
in
pkgs.dockerTools.buildLayeredImage {
  name = "d2b-wayland";
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
    d2bGatewayRelay
    msiTokenHelper
    footConfig
    pkgs.dockerTools.fakeNss # minimal /etc/passwd + /etc/group + nobody
  ];

  # World-writable /tmp + the runtime dir the agent uses.
  extraCommands = ''
    mkdir -p tmp run/d2b
    chmod 1777 tmp
    chmod 0700 run/d2b
  '';

  config = {
    Entrypoint = [ "${agent}/bin/d2b-sandbox-agent" ];
    Env = [
      "XDG_RUNTIME_DIR=/run/d2b"
      "XDG_CONFIG_DIRS=/etc/xdg"
      "FONTCONFIG_FILE=${fontsConf}"
      "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
      "LC_ALL=C.UTF-8"
      "D2B_APP=foot"
      "D2B_WP_COMPRESS=zstd"
      "TERM=xterm-256color"
    ];
    Labels = {
      "org.d2b.component" = "aca-wayland-poc";
      "org.d2b.adr" = "0032";
    };
  };
}
