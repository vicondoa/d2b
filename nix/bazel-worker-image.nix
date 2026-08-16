{ pkgs, bazel, system }:

let
  platforms = {
    x86_64-linux = "linux-x86_64";
    aarch64-linux = "linux-aarch64";
  };
  platform =
    if builtins.hasAttr system platforms
    then builtins.getAttr system platforms
    else throw "unsupported Bazel worker system: ${system}";
  contract = {
    schemaVersion = 1;
    contract = "d2b-bazel-worker/v1";
    inherit platform system;
    bazelVersion = bazel.version;
    toolchain = "rules_rust";
    targetIdentity = "//...";
    uid = 1000;
    capabilities = [ ];
    network = "disabled";
    featureSet = [
      "remote-download-outputs-minimal"
      "experimental-remote-features-disabled"
    ];
  };
  contractJson = builtins.toJSON contract;
  contractDigest =
    "sha256:${builtins.hashString "sha256" contractJson}";
in
pkgs.runCommand "d2b-bazel-worker-image-${platform}" {
  nativeBuildInputs = [ pkgs.coreutils ];
} ''
  set -euo pipefail
  mkdir -p "$out"
  cat > "$out/worker-image.json" <<'EOF'
${builtins.toJSON (contract // { digest = contractDigest; })}
EOF
''
