{ pkgs, flakeRoot, system, ... }:

let
  lib = pkgs.lib;
  hostBroker = builtins.readFile (flakeRoot + "/nixos-modules/host-broker.nix");
  flake = builtins.readFile (flakeRoot + "/flake.nix");
  guestDeny =
    builtins.readFile (flakeRoot + "/packages/d2b-guest-shell-runner/deny.toml");
  rustGate = builtins.readFile (flakeRoot + "/tests/test-rust.sh");
  toolchain = builtins.fromJSON (builtins.readFile
    (flakeRoot + "/tests/golden/bazel-toolchain.json"));
  supervisor = builtins.fromJSON (builtins.readFile
    (flakeRoot + "/tests/golden/bazel-exec-supervisor.json"));
  has = text: needle: lib.hasInfix needle text;
  all = text: needles: builtins.all (needle: has text needle) needles;
  replace = text: old: new: builtins.replaceStrings [ old ] [ new ] text;
  hash = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
  contexts = [
    "x86_64-linux/x86_64-unknown-linux-gnu/broker-production"
    "x86_64-linux/x86_64-unknown-linux-musl/guest-real-libshpool"
    "aarch64-linux/aarch64-unknown-linux-gnu/broker-production"
    "aarch64-linux/aarch64-unknown-linux-musl/guest-real-libshpool"
  ];
  checks = [
    "broker-production-dependency-policy"
    "guest-shell-runner-static-dependency-policy"
    "broker-production-package-policy"
    "guest-real-libshpool-package-policy"
    "broker-host-artifact-contract"
    "guest-static-elf"
  ];
in
{
  "bazel-package-policy/root-source-and-lock" = {
    expr = all hostBroker [
      "src = packagesSrc;"
      "lockFile = ../packages/Cargo.lock;"
      "\"--package\""
      "\"d2b-priv-broker\""
      "\"--bin\""
      "\"d2b-priv-broker\""
      "\"--no-default-features\""
    ] && !(has hostBroker "sourceRoot = \"source/d2b-priv-broker\"")
      && !(has hostBroker "../packages/d2b-priv-broker/Cargo.lock");
    expected = true;
  };

  "bazel-package-policy/guest-root-source-and-lock" = {
    expr = all flake [
      "src = rustPackagesSrc;"
      "sourceRoot = \"d2b-rust-src/packages\";"
      "lockFile = ./packages/Cargo.lock;"
      "\"--package\""
      "\"d2b-guest-shell-runner\""
      "\"--bin\""
      "\"d2b-guest-shell-runner\""
      "\"--no-default-features\""
      "\"--features\""
      "\"real-libshpool\""
    ] && !(has flake "./packages/d2b-guest-shell-runner/Cargo.lock");
    expected = true;
  };

  "bazel-package-policy/exact-wl-proxy-pins" = {
    expr =
      (lib.count (needle: needle == hash)
        (lib.splitString hash hostBroker) == 1)
      && lib.hasInfix hash flake;
    expected = true;
  };

  "bazel-package-policy/four-native-contexts" = {
    expr = builtins.all
      (context: has rustGate context)
      contexts
      && has flake "x86_64-linux"
      && has flake "aarch64-linux"
      && has flake "packages/policy-inputs";
    expected = true;
  };

  "bazel-package-policy/six-native-checks" = {
    expr = builtins.all (check: has flake check) checks;
    expected = true;
  };

  "bazel-package-policy/generic-exclusions" = {
    expr =
      has flake "\"--exclude\""
      && has flake "\"d2b-priv-broker\""
      && has flake "\"d2b-guest-shell-runner\""
      && has flake "cargo clippy --workspace --all-targets";
    expected = true;
  };

  "bazel-package-policy/guest-elf-contract" = {
    expr = all flake [
      "ET_DYN"
      "EM_X86_64"
      "EM_AARCH64"
      "PT_INTERP"
      "DT_NEEDED"
      "readelf"
      "selectedPolicyDigest"
      "sizeGrowthAuthorization"
    ] && !(has flake "/nix/store/");
    expected = true;
  };

  "bazel-package-policy/no-deleted-lock-inputs" = {
    expr = !(has flake "packages/d2b-priv-broker/Cargo.lock")
      && !(has flake "packages/d2b-guest-shell-runner/Cargo.lock")
      && !(has hostBroker "packages/d2b-priv-broker/Cargo.lock")
      && has flake "packages/Cargo.lock"
      && has flake "packages/Cargo.guest.lock";
    expected = true;
  };

  "bazel-package-policy/six-license-exceptions" = {
    expr = all guestDeny [
      "[licenses.exceptions]"
      "bindgen"
      "BSD-3-Clause"
      "instant"
      "inotify"
      "inotify-sys"
      "libloading"
      "notify"
      "ISC"
      "CC0-1.0"
    ] && !(has guestDeny "allow = [\n    \"Apache-2.0\",\n    \"BSD-3-Clause\"")
      && !(has guestDeny "allow = [\n    \"Apache-2.0\",\n    \"ISC\"")
      && !(has guestDeny "allow = [\n    \"Apache-2.0\",\n    \"CC0-1.0\"");
    expected = true;
  };

  "bazel-package-policy/no-fetch-and-independent-aggregates" = {
    expr = has flake "--no-fetch"
      && has rustGate "--no-fetch"
      && has flake "run_audit ${rustPackagesSrc}/packages/Cargo.lock"
      && has flake "run_audit ${rustPackagesSrc}/packages/Cargo.guest.lock"
      && has flake "guest-real-libshpool/production/closure.json"
      && has flake "guest-real-libshpool/production/Cargo.lock"
      && !(has flake "d2b-guest-shell-runner/Cargo.lock");
    expected = true;
  };

  "bazel-package-policy/prep-identities-remain-bound" = {
    expr = toolchain.schemaVersion == 1
      && supervisor.schemaVersion == 1
      && toolchain.derivationSha256Method == "raw-drv-file-sha256"
      && supervisor.derivationSha256Method == "raw-drv-file-sha256"
      && supervisor.protocol.privateExecutableFd == 9
      && supervisor.protocol.statusFd == 8
      && !(has (builtins.toJSON toolchain) "/nix/store/")
      && !(has (builtins.toJSON supervisor) "/nix/store/");
    expected = true;
  };

  "bazel-package-policy/missing-hash-mutation" = {
    expr = !(has (replace hostBroker hash "") hash);
    expected = false;
  };

  "bazel-package-policy/wrong-hash-mutation" = {
    expr = has (replace hostBroker hash "sha256-wrong") hash;
    expected = false;
  };

  "bazel-package-policy/one-sided-hash-mutation" = {
    expr = (has hostBroker hash) && !(has (replace flake hash "") hash);
    expected = false;
  };

  "bazel-package-policy/non-pie-mutation" = {
    expr = has (replace flake "ET_DYN" "ET_EXEC") "ET_DYN";
    expected = false;
  };

  "bazel-package-policy/wrong-machine-mutation" = {
    expr = has (replace flake "EM_AARCH64" "EM_X86_64") "EM_AARCH64";
    expected = false;
  };
}
