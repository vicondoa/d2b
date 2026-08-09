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
  nativeManifest = builtins.fromJSON (builtins.readFile
    (flakeRoot + "/tests/golden/native-policy-check-manifest.json"));
  policyPath = flakeRoot
    + "/packages/policy-inputs/x86_64-linux/x86_64-unknown-linux-musl/guest-real-libshpool";
  policyArtifact = builtins.fromJSON (builtins.readFile
    (policyPath + "/policy/metadata.json"));
  policyLock = builtins.fromTOML (builtins.readFile
    (policyPath + "/policy/Cargo.lock"));
  has = text: needle: builtins.replaceStrings [ needle ] [ "" ] text != text;
  all = text: needles: builtins.all (needle: has text needle) needles;
  replace = text: old: new: builtins.replaceStrings [ old ] [ new ] text;
  hash = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
  contexts = map (context: context.policyInput) nativeManifest.contexts;
  checks = nativeManifest.nativeChecks;
  policyArtifactShapeOk =
    (import (flakeRoot + "/nixos-modules/policy-artifact-validator.nix") { inherit lib; })
      .policyArtifactShapeOk;
  get = name: value:
    if builtins.isAttrs value && builtins.hasAttr name value
    then builtins.getAttr name value
    else null;
  policyValid = artifact: lock:
    policyArtifactShapeOk {
      inherit artifact lock;
      expected = {
        system = "x86_64-linux";
        target = "x86_64-unknown-linux-musl";
        package = "d2b-guest-shell-runner";
        features = [ "real-libshpool" ];
        defaultFeatures = false;
      };
      variant = "policy";
      expectedEdgeKinds = "normal,build,dev";
    };
  replaceRootNode = artifact: transform:
    artifact // {
      resolve = artifact.resolve // {
        nodes = map
          (node: if node.id == artifact.resolve.root then transform node else node)
          artifact.resolve.nodes;
      };
    };
  rootNode = lib.findFirst
    (node: node.id == policyArtifact.resolve.root)
    null
    policyArtifact.resolve.nodes;
  firstDetail = builtins.head rootNode.deps;
  alternatePackageId = builtins.head
    (lib.filter (id: id != firstDetail.pkg)
      (map (package: package.id) policyArtifact.packages));
  omittedEdge = replaceRootNode policyArtifact
    (node: node // {
      dependencies = builtins.tail node.dependencies;
    });
  wrongEdge = replaceRootNode policyArtifact
    (node: node // {
      deps = [ ((builtins.head node.deps) // {
        pkg = alternatePackageId;
      }) ] ++ (builtins.tail node.deps);
    });
  wrongKind = replaceRootNode policyArtifact
    (node: node // {
      deps = [ ((builtins.head node.deps) // {
        dep_kinds = [ { kind = "unexpected"; target = null; } ];
      }) ] ++ (builtins.tail node.deps);
    });
  lockIdentityMutation = policyLock // {
    package = [
      ((builtins.head policyLock.package) // { version = "0.0.0-mutated"; })
    ] ++ (builtins.tail policyLock.package);
  };
  authorizationRow = {
    system = "x86_64-linux";
    artifact = "guest-static-elf";
    binaryBytes = 100;
  };
  currentBinaryDigest =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
  rationaleDigest = builtins.hashFile "sha256" (flakeRoot + "/flake.nix");
  authorization = {
    system = authorizationRow.system;
    artifact = authorizationRow.artifact;
    priorBinaryBytes = authorizationRow.binaryBytes;
    newBinaryBytes = 107;
    deltaBytes = 7;
    rationalePath = "flake.nix";
    candidateContentSha256 = currentBinaryDigest;
    reviewRecordSha256 = rationaleDigest;
    decision = "approved";
  };
  authorizationValid = row: realizedBytes: realizedDigest: value:
    let
      keys = [
        "system"
        "artifact"
        "priorBinaryBytes"
        "newBinaryBytes"
        "deltaBytes"
        "rationalePath"
        "candidateContentSha256"
        "reviewRecordSha256"
        "decision"
      ];
      path = if builtins.isAttrs value then get "rationalePath" value else null;
      components =
        if builtins.isString path then lib.splitString "/" path else [ ];
      pathOk =
        builtins.isString path
        && path != ""
        && !(lib.hasPrefix "/" path)
        && builtins.all
          (component: component != "" && component != "." && component != "..")
          components
        && builtins.pathExists (flakeRoot + "/${path}");
    in
    if value == null then realizedBytes <= row.binaryBytes else
    builtins.isAttrs value
    && lib.sort builtins.lessThan (builtins.attrNames value)
      == lib.sort builtins.lessThan keys
    && value.system == row.system
    && value.artifact == row.artifact
    && value.priorBinaryBytes == row.binaryBytes
    && value.newBinaryBytes == realizedBytes
    && value.deltaBytes == realizedBytes - row.binaryBytes
    && realizedBytes > row.binaryBytes
    && value.decision == "approved"
    && pathOk
    && value.candidateContentSha256 == realizedDigest
    && value.reviewRecordSha256 == builtins.hashFile "sha256"
      (flakeRoot + "/${path}");
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
    expr = has hostBroker hash && has flake hash;
    expected = true;
  };

  "bazel-package-policy/four-native-contexts" = {
    expr = builtins.length contexts == 4
      && has rustGate "native-policy-check-manifest.json"
      && has flake "native-policy-check-manifest.json"
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
      "binarySha256"
      "actual_binary_sha"
      "sha256sum \"$binary\""
      "selectedPolicyDigest"
      "sizeGrowthAuthorization"
      "builtins.fromJSON"
      "builtins.fromTOML"
      "D2B_BAZEL_EXEC_SUPERVISOR"
    ] && !(has flake "/nix/store/");
    expected = true;
  };

  "bazel-package-policy/nix-policy-structural-positive" = {
    expr = policyValid policyArtifact policyLock;
    expected = true;
  };

  "bazel-package-policy/nix-policy-wrong-context" = {
    expr = policyValid (policyArtifact // { system = "aarch64-linux"; }) policyLock;
    expected = false;
  };

  "bazel-package-policy/nix-policy-wrong-target" = {
    expr = policyValid
      (policyArtifact // { target = "x86_64-unknown-linux-gnu"; })
      policyLock;
    expected = false;
  };

  "bazel-package-policy/nix-policy-wrong-root" = {
    expr = policyValid (policyArtifact // { root = "d2b-priv-broker"; }) policyLock;
    expected = false;
  };

  "bazel-package-policy/nix-policy-duplicate-root" = {
    expr = policyValid (policyArtifact // {
      packages = [ (builtins.head policyArtifact.packages) ]
        ++ policyArtifact.packages;
    }) policyLock;
    expected = false;
  };

  "bazel-package-policy/nix-policy-empty-identity-graph" = {
    expr = policyValid (policyArtifact // {
      identities = [ ];
      packages = [ ];
      resolve = policyArtifact.resolve // { nodes = [ ]; };
    }) policyLock;
    expected = false;
  };

  "bazel-package-policy/nix-policy-omitted-edge" = {
    expr = policyValid omittedEdge policyLock;
    expected = false;
  };

  "bazel-package-policy/nix-policy-misplaced-edge" = {
    expr = policyValid wrongEdge policyLock;
    expected = false;
  };

  "bazel-package-policy/nix-policy-wrong-edge-kind" = {
    expr = policyValid wrongKind policyLock;
    expected = false;
  };

  "bazel-package-policy/nix-policy-wrong-feature" = {
    expr = policyValid (policyArtifact // { features = [ ]; }) policyLock;
    expected = false;
  };

  "bazel-package-policy/nix-policy-default-feature-mutation" = {
    expr = policyValid (policyArtifact // { defaultFeatures = true; }) policyLock;
    expected = false;
  };

  "bazel-package-policy/nix-policy-variant-mutation" = {
    expr = policyValid (policyArtifact // { variant = "production"; }) policyLock;
    expected = false;
  };

  "bazel-package-policy/nix-policy-closed-edge-mutation" = {
    expr = policyValid
      (replaceRootNode policyArtifact
        (node: node // {
          dependencies = node.dependencies ++ [ "missing-package-id" ];
        }))
      policyLock;
    expected = false;
  };

  "bazel-package-policy/nix-policy-lock-identity-mutation" = {
    expr = policyValid policyArtifact lockIdentityMutation;
    expected = false;
  };

  "bazel-package-policy/size-authorization-null-positive" = {
    expr = authorizationValid authorizationRow 100 currentBinaryDigest null;
    expected = true;
  };

  "bazel-package-policy/size-authorization-positive" = {
    expr = authorizationValid authorizationRow 107 currentBinaryDigest authorization;
    expected = true;
  };

  "bazel-package-policy/size-authorization-stale-candidate" = {
    expr = authorizationValid authorizationRow 107
      "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
      authorization;
    expected = false;
  };

  "bazel-package-policy/size-authorization-stale-review" = {
    expr = authorizationValid authorizationRow 107 currentBinaryDigest
      (authorization // {
        reviewRecordSha256 =
          "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
      });
    expected = false;
  };

  "bazel-package-policy/size-authorization-cross-artifact" = {
    expr = authorizationValid authorizationRow 107 currentBinaryDigest
      (authorization // { artifact = "broker-host-artifact-contract"; });
    expected = false;
  };

  "bazel-package-policy/size-authorization-wrong-delta" = {
    expr = authorizationValid authorizationRow 107 currentBinaryDigest
      (authorization // { deltaBytes = 8; });
    expected = false;
  };

  "bazel-package-policy/size-authorization-absolute-rationale" = {
    expr = authorizationValid authorizationRow 107 currentBinaryDigest
      (authorization // { rationalePath = "/tmp/review.md"; });
    expected = false;
  };

  "bazel-package-policy/size-authorization-missing-rationale" = {
    expr = authorizationValid authorizationRow 107 currentBinaryDigest
      (authorization // { rationalePath = "reviews/missing.md"; });
    expected = false;
  };

  "bazel-package-policy/size-authorization-duplicate-authority" = {
    expr =
      let authorities = [ authorization authorization ];
      in lib.length (lib.unique (map builtins.toJSON authorities))
        == lib.length authorities;
    expected = false;
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
    expr = has guestDeny "exceptions = ["
      && all guestDeny [
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
      && has flake "run_audit \${rustPackagesSrc}/packages/Cargo.lock"
      && has flake "run_audit \${rustPackagesSrc}/packages/Cargo.guest.lock"
      && has flake "policyContextRoot"
      && has flake "/production/closure.json"
      && has flake "/production/Cargo.lock"
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
    expr = has (replace hostBroker hash "") hash;
    expected = false;
  };

  "bazel-package-policy/wrong-hash-mutation" = {
    expr = has (replace hostBroker hash "sha256-wrong") hash;
    expected = false;
  };

  "bazel-package-policy/one-sided-hash-mutation" = {
    expr = (has hostBroker hash) && has (replace flake hash "") hash;
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
