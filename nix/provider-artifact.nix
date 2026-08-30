{ pkgs, lib ? pkgs.lib }:

{ artifactId
, binary
, binaryRef
, manifest
, signature
, configSchema
, publicKey
, packageName ? "d2b-provider-${artifactId}"
, providerName ? artifactId
, version ? "0.0.0"
, packageDigestOverride ? null
, systems ? [ pkgs.system ]
, platform ? pkgs.system
, apiCompatibility ? "d2b.zone.v3"
, serviceCompatibility ? "d2bd.resource"
, componentDigestOverride ? null
, descriptorDigestOverride ? null
, supportContact ? "provider-support"
, signatureId ? "default"
, revocationStatus ? "clear"
, denyStatus ? "clear"
, provenanceEvidence ? "accepted"
, sbomEvidence ? "accepted"
, licenseEvidence ? "accepted"
, vulnerabilityEvidence ? "accepted"
, conformanceAttestation ? "accepted"
, supportChannel ? "stable"
}:

let
  digestPattern = "sha256:[0-9a-f]{64}";
  binaryPattern = "^[a-z][a-z0-9-]*$";
  rawDigest = path: "sha256:${builtins.hashFile "sha256" path}";
  manifestData = builtins.fromJSON (builtins.readFile manifest);
  publisher = manifestData.trust.publisher;
  controllers = lib.filter
    (component: (component.componentType or null) == "controller")
    (manifestData.components or [ ]);
  controller =
    if lib.length controllers == 1
    then lib.head controllers
    else null;
  controllerTypes =
    if controller == null
    then [ ]
    else controller.exportedResourceTypes or [ ];
  controllerBindings = lib.filter
    (binding: builtins.elem (binding.resourceType or null) controllerTypes)
    (manifestData.apiBindings or [ ]);
  placementAnchor =
    if controllerBindings == [ ]
    then null
    else (lib.head controllerBindings).placementAnchor or null;
  runtimeArtifacts = manifestData.runtimeArtifacts or [ ];
  runtime =
    if runtimeArtifacts == [ ]
    then null
    else lib.head runtimeArtifacts;
  executableSetDigestFile = pkgs.runCommand "d2b-provider-${artifactId}-executable-digest" {
    nativeBuildInputs = [ pkgs.python3 ];
  } ''
    set -euo pipefail
    python3 - "${binary}/bin/${binaryRef}" "$out" <<'PY'
    import hashlib
    import json
    import pathlib
    import sys

    binary_path, output_path = sys.argv[1:]
    binary_digest = "sha256:" + hashlib.sha256(
        pathlib.Path(binary_path).read_bytes()
    ).hexdigest()
    executable_map = json.dumps(
        {pathlib.Path(binary_path).name: binary_digest},
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    pathlib.Path(output_path).write_text(
        "sha256:" + hashlib.sha256(
            b"d2b:v3:provider-executable-set\0" + executable_map
        ).hexdigest(),
        encoding="ascii",
    )
    PY
  '';
  executableSetDigest =
    lib.removeSuffix "\n" (builtins.readFile executableSetDigestFile);
  manifestExecutableDigest = manifestData.digests.executable or null;
  targetCapabilityDigests = lib.concatMap
    (component:
      map (capability: capability.artifactDigest or null)
        (component.targetCapabilities or [ ]))
    (manifestData.components or [ ]);
  manifestDigest = rawDigest manifest;
  configDigest = rawDigest configSchema;
  componentSetDigest = "sha256:${builtins.hashString "sha256"
    (builtins.toJSON (manifestData.components or [ ]))}";
  descriptorSetDigest = "sha256:${builtins.hashString "sha256"
    (builtins.toJSON (manifestData.apiBindings or [ ]))}";
  escapeShellArg = lib.escapeShellArg;
  assembled = pkgs.runCommand "d2b-provider-${artifactId}" {
    nativeBuildInputs = [ pkgs.coreutils ];
  } ''
    set -euo pipefail
    binary_name=${escapeShellArg binaryRef}

    install -Dm755 "${binary}/bin/${binaryRef}" \
      "$out/bin/$binary_name"
    install -Dm644 "${manifest}" \
      "$out/share/d2b/provider/provider-manifest.json"
    install -Dm644 "${signature}" \
      "$out/share/d2b/provider/provider-manifest.json.sig"
    install -Dm644 "${configSchema}" \
      "$out/share/d2b/provider/config-schema.json"

    test -x "$out/bin/$binary_name"
    test "$(wc -c < "$out/share/d2b/provider/provider-manifest.json.sig")" -eq 64
    expected_package_digest=${escapeShellArg
      (if packageDigestOverride == null then "" else packageDigestOverride)}
    if [ -n "$expected_package_digest" ]; then
      test "sha256:$(${pkgs.nix}/bin/nix --extra-experimental-features nix-command \
        hash path --type sha256 --base16 "$out")" = "$expected_package_digest"
    fi

    actual="$(find "$out" -type f -printf '%P\n' | sort)"
    expected="$(printf '%s\n' \
      "bin/$binary_name" \
      "share/d2b/provider/config-schema.json" \
      "share/d2b/provider/provider-manifest.json" \
      "share/d2b/provider/provider-manifest.json.sig" | sort)"
    test "$actual" = "$expected"
  '';
  packageDigestFile = pkgs.runCommand "d2b-provider-${artifactId}-nar-digest" {
    nativeBuildInputs = [ pkgs.nix ];
  } ''
    set -euo pipefail
    printf 'sha256:%s' \
      "$(${pkgs.nix}/bin/nix --extra-experimental-features nix-command \
        hash path --type sha256 --base16 "${assembled}")" > "$out"
  '';
  computedPackageDigest =
    lib.removeSuffix "\n" (builtins.readFile packageDigestFile);
  packageDigest =
    if packageDigestOverride == null
    then computedPackageDigest
    else packageDigestOverride;
  componentDigest =
    if componentDigestOverride == null
    then componentSetDigest
    else componentDigestOverride;
  descriptorDigest =
    if descriptorDigestOverride == null
    then descriptorSetDigest
    else descriptorDigestOverride;
  catalog = {
    inherit
      providerName
      packageName
      publisher
      version
      packageDigest
      manifestDigest
      configDigest
      systems
      platform
      apiCompatibility
      serviceCompatibility
      componentDigest
      descriptorDigest
      supportContact
      revocationStatus
      denyStatus
      provenanceEvidence
      sbomEvidence
      licenseEvidence
      vulnerabilityEvidence
      conformanceAttestation
      supportChannel
    ;
    executableDigest = executableSetDigest;
    rootEpoch = manifestData.trust.rootEpoch or 1;
    signature = { inherit signatureId; };
  }
  // lib.optionalAttrs (controller != null) {
    instanceScope = controller.instanceScope or null;
    supportedTargetKinds = controller.supportedTargetKinds or [ ];
    targetCapabilities = controller.targetCapabilities or [ ];
    inherit placementAnchor;
  }
  // lib.optionalAttrs (runtime != null) {
    d2bdDigest = runtime.d2bdDigest or null;
    brokerDigest = runtime.brokerDigest or null;
  };
  trustedPublisher = {
    publisherRef = publisher;
    signingKey = builtins.readFile publicKey;
  };
  package = assembled // {
    passthru = (assembled.passthru or { }) // {
      providerArtifact = {
        inherit catalog trustedPublisher;
      };
    };
  };
  descriptor = {
    inherit package;
    type = "provider";
    inherit catalog;
  };
in
assert builtins.isString artifactId
  && builtins.stringLength artifactId <= 64
  && builtins.match binaryPattern artifactId != null;
assert manifestData.artifactId == artifactId;
assert builtins.isString binaryRef
  && builtins.match binaryPattern binaryRef != null;
assert builtins.isString publisher
  && builtins.match binaryPattern publisher != null;
assert builtins.isString packageDigest
  && builtins.match digestPattern packageDigest != null;
assert packageDigestOverride == null
  || packageDigestOverride == computedPackageDigest;
assert builtins.isString executableSetDigest
  && builtins.match digestPattern executableSetDigest != null;
assert manifestExecutableDigest == executableSetDigest;
assert lib.all
  (digest: digest == executableSetDigest)
  targetCapabilityDigests;
assert builtins.isString manifestDigest
  && builtins.match digestPattern manifestDigest != null;
assert builtins.isString configDigest
  && builtins.match digestPattern configDigest != null;
assert builtins.isList systems
  && lib.length systems > 0
  && lib.all (system: builtins.isString system) systems;
{
  inherit package;
  inherit catalog descriptor;
  inherit trustedPublisher;
}
