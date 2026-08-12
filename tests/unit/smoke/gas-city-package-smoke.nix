{ pkgs
, gasCityContributor
, gascityRevision
, gascityVersion
, gascityPacksRevision
, beadsRevision
, beadsVersion
, llmAgentsRevision
, packageNixpkgsRevision
, copilotVersion
, goVersion
, bazelVersion
, doltVersion
}:

let
  runtimeScripts = gasCityContributor.passthru.runtimeScripts;
  managedAssetsFor = generation:
    pkgs.runCommand "gas-city-managed-assets-${generation}" { } ''
      set -euo pipefail
      for name in city pack copilot buildbuddy; do
        mkdir -p "$out/$name"
        printf '%s\n' "${generation}-$name" > "$out/$name/fixture"
      done
    '';
  oldManagedAssets = managedAssetsFor "old-generation";
  newManagedAssets = managedAssetsFor "new-generation";
in
pkgs.runCommand "gas-city-package-smoke" {
  nativeBuildInputs = [
    pkgs.coreutils
    pkgs.findutils
    pkgs.gnugrep
    pkgs.jq
    pkgs.python3
  ];
} ''
  set -euo pipefail

  export HOME="$TMPDIR/home"
  export PATH="${gasCityContributor}/bin"
  mkdir -p "$HOME"

  root="${gasCityContributor}/share/gas-city-contributor"
  fdproxy="${runtimeScripts}/bin/gascity-fdproxy"
  test -L "${gasCityContributor}/bin/gascity-fdproxy"
  test "$(readlink "${gasCityContributor}/bin/gascity-fdproxy")" = "$fdproxy"
  test -x "$fdproxy"
  test ! -L "$fdproxy"

  # Every executable used by the contributor boundary must come from the
  # immutable closure, not from the evaluator's or operator's ambient PATH.
  for tool in \
    gc bd dolt copilot go bazel bwrap nft tinyproxy envoy python3 \
    git gh openssl jq ps lsof flock nix
  do
    toolPath="${gasCityContributor}/bin/$tool"
    test -x "$toolPath"
    test "$(command -v "$tool")" = "$toolPath"
  done
  test -L "${gasCityContributor}/bin/openssl"
  opensslTarget="$(${pkgs.coreutils}/bin/readlink -f \
    "${gasCityContributor}/bin/openssl")"
  case "$opensslTarget" in
    /nix/store/*) ;;
    *) echo "OpenSSL is outside the immutable closure" >&2; exit 1 ;;
  esac
  gitTarget="$(${pkgs.coreutils}/bin/readlink -f \
    "${gasCityContributor}/bin/git")"
  case "$gitTarget" in
    /nix/store/*) ;;
    *) echo "Git is outside the immutable closure" >&2; exit 1 ;;
  esac
  test -r "${gasCityContributor}/etc/ssl/certs/ca-bundle.crt"

  gc_version="$(${gasCityContributor}/bin/gc version --long)"
  printf '%s\n' "$gc_version" | grep -F "commit: ${gascityRevision}"
  printf '%s\n' "$(${gasCityContributor}/bin/go version)" \
    | grep -F "go${goVersion}"
  printf '%s\n' "$(${gasCityContributor}/bin/bazel --version)" \
    | grep -F "${bazelVersion}"
  printf '%s\n' "$(${gasCityContributor}/bin/copilot --version)" \
    | grep -F "${copilotVersion}"
  printf '%s\n' "$(${gasCityContributor}/bin/dolt version)" \
    | grep -F "${doltVersion}"
  printf '%s\n' "$(${gasCityContributor}/bin/bd version)" \
    | grep -F "${beadsRevision}"

  # The compare-and-set flags are the continuation contract used by the
  # decision workflow; checking help avoids creating a mutable beads store.
  ${gasCityContributor}/bin/bd update --help \
    | grep -F -- "--if-assignee"
  ${gasCityContributor}/bin/bd update --help \
    | grep -F -- "--if-status"

  # Gas City validates every immutable pack without credentials.  The
  # recursive "." form is the pinned CLI's supported syntax; the upstream
  # GitHub pack is intentionally not part of this U1 closure.
  (
    cd "$root/packs"
    ${gasCityContributor}/bin/gc lint .
  )
  ${gasCityContributor}/bin/gc \
    --city "$root/city" \
    config show > "$TMPDIR/resolved-city.toml"
  grep -F '/var/lib/gascity-contributor/state/rigs/d2b' \
    "$TMPDIR/resolved-city.toml"
  grep -F 'requirements-planner' "$TMPDIR/resolved-city.toml"
  grep -F 'name = "ce-work"' "$TMPDIR/resolved-city.toml"
  test ! -e "$root/packs/github"
  test -x "${gasCityContributor}/bin/gascity-discord-decision"
  test -x "${gasCityContributor}/bin/gascity-publish-pr"
  test -x "${gasCityContributor}/bin/gascity-service-activation"
  test -x "${gasCityContributor}/bin/gascity-check"
  test -x "${gasCityContributor}/bin/gascity-check-runner"
  test -f "$root/pack/formulas/d2b-decision.formula.toml"
  test -f "$root/pack/assets/workflows/d2b-decision/request.md"
  test -f "$root/pack/assets/workflows/d2b-decision/wait.md"
  test -f "$root/pack/assets/workflows/d2b-contributor-build/publish.md"
  grep -F 'write-terminal-state' \
    "$root/pack/assets/workflows/d2b-compound-resolution/{target}.md"

  discord_pack="$root/packs/discord/pack.toml"
  grep -F 'name = "discord-gateway"' "$discord_pack"
  grep -F 'visibility = "private"' "$discord_pack"
  ! grep -E 'discord-interactions|discord-admin|visibility = "(public|tenant)"' \
    "$discord_pack"

  # Compile pack scripts without writing bytecode into the immutable store.
  while IFS= read -r -d "" script; do
    PYTHONPYCACHEPREFIX="$TMPDIR/pycache" \
      ${gasCityContributor}/bin/python3 -m py_compile "$script"
  done < <(find "$root/packs" -type f -name "*.py" -print0)
  for script in \
    "$root/pack/scripts/discord-decision.py" \
    "$root/pack/scripts/publish-pr.py" \
    "$root/pack/scripts/check-runner.py" \
    "$root/pack/scripts/copilot-profile.py" \
    "$root/pack/scripts/agent-launcher.py" \
    "$root/pack/scripts/agent-sandbox.py" \
    "$root/pack/scripts/fdproxy.py" \
    "$root/pack/scripts/service-activation.py"
  do
    PYTHONPYCACHEPREFIX="$TMPDIR/pycache" \
      ${gasCityContributor}/bin/python3 -m py_compile "$script"
  done

  # The committed fixture suites derive their repository root from their
  # filesystem layout.  Preserve that layout in a writable temporary tree so
  # they execute against the contributor source and package scripts rather
  # than against the evaluator's checkout.
  fixtureRepo="$TMPDIR/gas-city-fixture-repo"
  mkdir -p "$fixtureRepo/tests/fixtures" "$fixtureRepo/nix" \
    "$fixtureRepo/nixos-modules"
  cp -R ${../../fixtures/gas-city} "$fixtureRepo/tests/fixtures/gas-city"
  cp -R ${../../../nix/gas-city-contributor} \
    "$fixtureRepo/nix/gas-city-contributor"
  cp -R ${../../../nixos-modules/gas-city-contributor} \
    "$fixtureRepo/nixos-modules/gas-city-contributor"
  ${pkgs.coreutils}/bin/chmod -R u+w "$fixtureRepo"
  mkdir -p "$fixtureRepo/.scratch"
  ${gasCityContributor}/bin/python3 \
    "$root/pack/scripts/service-activation.py" materialize-assets \
    --source "$root" \
    --destination "$fixtureRepo/.scratch/managed-package" \
    --uid "$(${pkgs.coreutils}/bin/id -u)" \
    --group "$(${pkgs.coreutils}/bin/id -g)"

  managedPackage="$fixtureRepo/.scratch/managed-package"
  test "$(${pkgs.coreutils}/bin/stat -c %u "$managedPackage")" = \
    "$(${pkgs.coreutils}/bin/id -u)"
  test "$(${pkgs.coreutils}/bin/stat -c %g "$managedPackage")" = \
    "$(${pkgs.coreutils}/bin/id -g)"
  test "$(${pkgs.coreutils}/bin/stat -c %a "$managedPackage")" = 750
  test -r "$managedPackage/city/city.toml"
  test -r "$managedPackage/pack/pack.toml"
  test -r "$managedPackage/copilot/code-luna/settings.json"
  test -r "$managedPackage/buildbuddy/envoy.yaml.tmpl"

  export PYTHONNOUSERSITE=1
  export PYTHONPYCACHEPREFIX="$TMPDIR/pycache"
  export XDG_CONFIG_HOME="$TMPDIR/config"
  export XDG_CACHE_HOME="$TMPDIR/cache"
  export GC_MANAGED_ASSET_OLD="${oldManagedAssets}"
  export GC_MANAGED_ASSET_NEW="${newManagedAssets}"
  unset COPILOT_GITHUB_TOKEN GITHUB_TOKEN DISCORD_TOKEN BUILD_BUDDY_API_KEY \
    GC_AGENT_LAUNCHER_TOKEN GC_FDPROXY_AUTH
  unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy
  export GIT_CONFIG_NOSYSTEM=1
  export GIT_CONFIG_GLOBAL=/dev/null
  export GIT_CONFIG_SYSTEM=/dev/null
  export GIT_TERMINAL_PROMPT=0
  export GIT_SSH_COMMAND=/bin/false
  export GIT_PROXY_COMMAND=/bin/false
  export GIT_ALLOW_PROTOCOL=https:file

  (
    cd "$fixtureRepo"
    GAS_CITY_FDPROXY="$fdproxy" \
      GC_TEST_MODE=1 \
      ${gasCityContributor}/bin/python3 \
      tests/fixtures/gas-city/acp/run.py
  )
  (
    cd "$fixtureRepo"
    ${gasCityContributor}/bin/python3 \
      tests/fixtures/gas-city/discord/test_router.py
  )
  (
    cd "$fixtureRepo"
    ${gasCityContributor}/bin/python3 \
      tests/fixtures/gas-city/github/test_publisher.py
  )
  (
    cd "$fixtureRepo"
    PATH=/definitely-not-a-command-path \
      GC_TEST_OPENSSL="${gasCityContributor}/bin/openssl" \
      GC_TEST_GIT="${gasCityContributor}/bin/git" \
      ${gasCityContributor}/bin/python3 \
      tests/fixtures/gas-city/github/test_publisher.py \
      PublisherFixture.test_github_api_signs_jwt_with_packaged_openssl_under_restricted_path
  )
  (
    cd "$fixtureRepo"
    PATH=/definitely-not-a-command-path \
      GC_TEST_GIT="${gasCityContributor}/bin/git" \
      ${gasCityContributor}/bin/python3 \
      tests/fixtures/gas-city/github/test_publisher.py \
      PublisherFixture.test_git_commands_use_explicit_executable_under_restricted_path_without_network
  )
  (
    cd "$fixtureRepo"
    GAS_CITY_ENVOY="${gasCityContributor}/bin/envoy" \
      GAS_CITY_CA_BUNDLE="${gasCityContributor}/etc/ssl/certs/ca-bundle.crt" \
      ${gasCityContributor}/bin/python3 \
      tests/fixtures/gas-city/buildbuddy/run.py
  )
  jq -e \
    --arg gascity "${gascityRevision}" \
    --arg gascityVersion "${gascityVersion}" \
    --arg gascityPacks "${gascityPacksRevision}" \
    --arg beads "${beadsRevision}" \
    --arg beadsVersion "${beadsVersion}" \
    --arg llmAgents "${llmAgentsRevision}" \
    --arg packageNixpkgs "${packageNixpkgsRevision}" \
    --arg copilot "${copilotVersion}" \
    --arg go "${goVersion}" \
    --arg bazel "${bazelVersion}" \
    --arg dolt "${doltVersion}" \
    '.gascity.revision == $gascity
     and .gascity.version == $gascityVersion
     and .gascity.source == "gastownhall/gascity"
     and .gascityPacks.revision == $gascityPacks
     and .gascityPacks.source == "gastownhall/gascity-packs"
     and .packageNixpkgs.revision == $packageNixpkgs
     and .packageNixpkgs.goVersion == $go
     and .packageNixpkgs.bazelVersion == $bazel
     and .llmAgents.revision == $llmAgents
     and .llmAgents.copilotVersion == $copilot
     and .dolt.source == "dolthub/dolt"
     and .dolt.version == $dolt
     and .beads.source == "steveyegge/beads"
     and .beads.revision == $beads
     and .beads.casRevision == $beads
     and .beads.version == $beadsVersion
     and .packs.discord == "gateway-only"
     and .packs.included == ["gascity", "compound-engineering", "discord"]
     and .packs.excluded == ["github"]
     and .runtime.certificateBundle == "etc/ssl/certs/ca-bundle.crt"
     and .runtime.requiredExecutables == [
       "gc", "bd", "dolt", "copilot", "go", "bazel", "bwrap",
       "nft", "tinyproxy", "envoy", "python3", "git", "gh", "openssl",
       "jq", "ps", "lsof", "flock", "nix"
     ]' \
    "$root/sources.json" >/dev/null

  mkdir -p "$out"
  printf '%s\n' "gas-city-package-smoke: ok" > "$out/result"
''
