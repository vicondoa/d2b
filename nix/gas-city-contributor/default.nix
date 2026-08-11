{ pkgs
, gascityPacksSrc
, gascity
, dolt
, beads
, copilot
, go
, bazel
, gascityRevision
, gascityPacksRevision
, beadsRevision
, llmAgentsRevision
, packageNixpkgsRevision
}:

let
  inherit (pkgs) lib;
  contributorRoot = ./.;
  requiredExecutables = [
    "gc"
    "bd"
    "dolt"
    "copilot"
    "go"
    "bazel"
    "bwrap"
    "nft"
    "tinyproxy"
    "envoy"
    "python3"
    "git"
    "gh"
    "openssl"
    "jq"
    "ps"
    "lsof"
    "flock"
    "nix"
  ];

  # Patch the complete source tree first, then select only the packs that are
  # part of this closure.  Keeping the sibling layout is required by
  # compound-engineering/pack.toml's ../gascity import.
  patchedPacks = pkgs.applyPatches {
    name = "gascity-packs-patched";
    src = gascityPacksSrc;
    patches = [ ./patches/discord-outbound-only.patch ];
  };

  runtimePackages = [
    gascity
    dolt
    beads
    copilot
    go
    bazel
    pkgs.bubblewrap
    pkgs.nftables
    pkgs.tinyproxy
    pkgs.envoy
    pkgs.cacert
    pkgs.openssl
    pkgs.procps
    pkgs.lsof
    pkgs.flock
    pkgs.nix
    pkgs.python3
    pkgs.git
    pkgs.bash
    pkgs.coreutils
    pkgs.findutils
    pkgs.gawk
    pkgs.gnugrep
    pkgs.gnused
    pkgs.which
    pkgs.gh
    pkgs.curl
    pkgs.jq
    pkgs.socat
    pkgs.openssh
  ];

  # buildEnv creates the executable namespace in declared path order.  Keep
  # duplicate names deterministic by retaining the first package and make
  # the certificate bundle part of the same immutable environment.
  runtimeEnvironment = pkgs.buildEnv {
    name = "gas-city-contributor-runtime";
    paths = runtimePackages;
    pathsToLink = [
      "/bin"
      "/etc/ssl/certs"
    ];
    ignoreCollisions = true;
  };

  # U2-U5 add immutable city, pack, profile, and integration files below this
  # directory.  The closure is intentionally forward-compatible with those
  # files while U1 remains independently evaluable.
  managedTree = name:
    let
      source = contributorRoot + "/${name}";
    in
    lib.optionalString (builtins.pathExists source) ''
      cp -R ${source} "$managedRoot/${name}"
    '';

  sourceManifest = builtins.toJSON {
    schemaVersion = 1;
    gascity = {
      revision = gascityRevision;
      source = "gastownhall/gascity";
      version = gascity.version;
    };
    gascityPacks = {
      revision = gascityPacksRevision;
      source = "gastownhall/gascity-packs";
    };
    llmAgents = {
      revision = llmAgentsRevision;
      source = "numtide/llm-agents.nix";
      copilotVersion = copilot.version;
    };
    packageNixpkgs = {
      revision = packageNixpkgsRevision;
      goVersion = go.version;
      bazelVersion = bazel.version;
    };
    dolt = {
      source = "dolthub/dolt";
      version = dolt.version;
    };
    beads = {
      casRevision = beadsRevision;
      revision = beadsRevision;
      source = "steveyegge/beads";
      version = beads.version;
    };
    packs = {
      discord = "gateway-only";
      excluded = [ "github" ];
      included = [ "gascity" "compound-engineering" "discord" ];
    };
    runtime = {
      certificateBundle = "etc/ssl/certs/ca-bundle.crt";
      inherit requiredExecutables;
    };
  };

  managedAssets = pkgs.runCommand "gas-city-contributor-assets" { } ''
    set -euo pipefail

    managedRoot="$out/share/gas-city-contributor"
    mkdir -p "$managedRoot/packs"
    cp -R ${patchedPacks}/gascity "$managedRoot/packs/gascity"
    cp -R ${patchedPacks}/compound-engineering \
      "$managedRoot/packs/compound-engineering"
    cp -R ${patchedPacks}/discord "$managedRoot/packs/discord"

    # These copies are immutable store inputs.  Runtime state (.gc, Dolt,
    # beads, worktrees, and caches) is deliberately not placed here.
    ${lib.concatMapStrings managedTree [ "city" "pack" "copilot" "buildbuddy" ]}

    printf '%s\n' '${sourceManifest}' > "$managedRoot/sources.json"
    printf '%s\n' \
      'The files under this directory are immutable contributor inputs.' \
      'Runtime state belongs below the service-owned state root.' \
      > "$managedRoot/README"
  '';
in
pkgs.symlinkJoin {
  name = "gas-city-contributor";
  paths = [
    runtimeEnvironment
    managedAssets
  ];
  passthru = {
    inherit
      gascity
      dolt
      beads
      copilot
      go
      bazel
      runtimePackages
      runtimeEnvironment
      requiredExecutables
      patchedPacks;
    revisions = {
      inherit
        gascityRevision
        gascityPacksRevision
        beadsRevision
        llmAgentsRevision
        packageNixpkgsRevision;
    };
  };
  meta = {
    description = "Pinned Gas City contributor runtime closure";
    platforms = lib.platforms.linux;
  };
}
