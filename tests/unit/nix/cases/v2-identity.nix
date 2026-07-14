{ flakeRoot, lib, ... }:

let
  identity = import (flakeRoot + "/nixos-modules/v2-identity.nix");
  vectors = builtins.fromJSON
    (builtins.readFile (flakeRoot + "/docs/reference/v2-identity-vectors.json"));

  computed = row:
    let
      encoded = identity.encode row.domain row.parts;
      digest = builtins.hashString "sha256" encoded;
    in
    {
      inherit encoded;
      encodedHex = identity.asciiHex encoded;
      sha256 = digest;
      shortId = identity.base32First96 digest;
      recomputed = (identity.recompute encoded).id;
    };

  expected = row: {
    inherit (row) encoded encodedHex sha256 shortId;
    recomputed = row.shortId;
  };

  attempt = value:
    (builtins.tryEval (builtins.deepSeq value true)).success;

  providerVectorValues = map
    (row: builtins.elemAt row.parts 1)
    (lib.filter
      (row:
        lib.hasPrefix "provider-" row.case
        && row.case != "provider-instance-rename")
      vectors.valid);

  roleVectorValues = map
    (row: builtins.elemAt row.parts 2)
    (lib.filter (row: lib.hasPrefix "role-" row.case) vectors.valid);

  nixMalformed = lib.filter (row: row ? encoded) vectors.malformed;
in
{
  "v2-identity/canonical-cross-language-vectors" = {
    expr = map computed vectors.valid;
    expected = map expected vectors.valid;
  };

  "v2-identity/partition-boundaries-are-distinct" = {
    expr = {
      rows = map
        (row:
          let
            encoded = identity.encode row.domain row.parts;
            digest = builtins.hashString "sha256" encoded;
          in
          {
            inherit encoded;
            encodedHex = identity.asciiHex encoded;
            sha256 = digest;
            shortId = identity.base32First96 digest;
          })
        vectors.partitionBoundary;
      encodingsDiffer =
        (builtins.elemAt vectors.partitionBoundary 0).encoded
        != (builtins.elemAt vectors.partitionBoundary 1).encoded;
      idsDiffer =
        (builtins.elemAt vectors.partitionBoundary 0).shortId
        != (builtins.elemAt vectors.partitionBoundary 1).shortId;
    };
    expected = {
      rows = map
        (row: {
          inherit (row) encoded encodedHex sha256 shortId;
        })
        vectors.partitionBoundary;
      encodingsDiffer = true;
      idsDiffer = true;
    };
  };

  "v2-identity/malformed-encodings-fail-recomputation" = {
    expr = map
      (row: {
        inherit (row) case;
        accepted = attempt (identity.recompute row.encoded);
      })
      nixMalformed;
    expected = map
      (row: {
        inherit (row) case;
        accepted = false;
      })
      nixMalformed;
  };

  "v2-identity/malformed-short-ids-are-rejected" = {
    expr = map
      (value: attempt (identity.validateShortId value))
      vectors.malformedShortIds;
    expected = map (_: false) vectors.malformedShortIds;
  };

  "v2-identity/provider-types-are-closed-and-fully-vectored" = {
    expr = {
      closed = identity.providerTypes;
      vectored = providerVectorValues;
    };
    expected = {
      closed = identity.providerTypes;
      vectored = identity.providerTypes;
    };
  };

  "v2-identity/role-kinds-are-closed-and-fully-vectored" = {
    expr = {
      closed = identity.roleKinds;
      vectored = roleVectorValues;
    };
    expected = {
      closed = identity.roleKinds;
      vectored = identity.roleKinds;
    };
  };

  "v2-identity/realm-path-is-leaf-to-root" = {
    expr = map
      (path: {
        inherit path;
        accepted = attempt (identity.validateRealmPath path);
      })
      [
        "local-root"
        "dev.local-root"
        "personal-dev.dev.local-root"
        "local-root.dev"
        "dev..local-root"
        "dev.local-root."
        "dev.local-root.d2b"
        "dévelop.local-root"
      ];
    expected = [
      { path = "local-root"; accepted = true; }
      { path = "dev.local-root"; accepted = true; }
      { path = "personal-dev.dev.local-root"; accepted = true; }
      { path = "local-root.dev"; accepted = false; }
      { path = "dev..local-root"; accepted = false; }
      { path = "dev.local-root."; accepted = false; }
      { path = "dev.local-root.d2b"; accepted = false; }
      { path = "dévelop.local-root"; accepted = false; }
    ];
  };

  "v2-identity/renames-produce-new-ids" = {
    expr =
      let
        realmA = identity.deriveRealmId "dev.local-root";
        realmB = identity.deriveRealmId "engineering.local-root";
        workloadA = identity.deriveWorkloadId realmA "personal-dev";
        workloadB = identity.deriveWorkloadId realmA "personal-dev-next";
        providerA = identity.deriveProviderId realmA "runtime" "primary";
        providerB = identity.deriveProviderId realmA "runtime" "secondary";
        roleA = identity.deriveRoleId realmA workloadA "cloud-hypervisor";
        roleB = identity.deriveRoleId realmA workloadA "qemu-media";
      in
      [
        (realmA != realmB)
        (workloadA != workloadB)
        (providerA != providerB)
        (roleA != roleB)
      ];
    expected = [ true true true true ];
  };

  "v2-identity/duplicate-provider-ids-fail-closed" = {
    expr = attempt (identity.validateGlobalIdentities {
      providers = [
        "aaaaaaaaaaaaaaaaaaaa"
        "aaaaaaaaaaaaaaaaaaaa"
      ];
    });
    expected = false;
  };

  "v2-identity/global-short-id-collisions-fail-closed" = {
    expr = attempt (identity.validateGlobalIdentities {
      realms = [ "aaaaaaaaaaaaaaaaaaaa" ];
      workloads = [ "aaaaaaaaaaaaaaaaaaaa" ];
    });
    expected = false;
  };

  "v2-identity/short-id-path-proof" = {
    expr = {
      inherit (identity) linuxUnixPathMaxBytes shortIdLength;
      vectorLength = vectors.shortIdProof.lengthBytes;
      vectorContainsNul = vectors.shortIdProof.containsNul;
      singleIdHeadroom = identity.unixPathHeadroom
        (builtins.elemAt vectors.valid 0).shortId;
      maxPathHeadroom = identity.unixPathHeadroom
        (lib.concatStrings (builtins.genList (_: "x") 107));
      overlongAccepted = attempt (identity.unixPathHeadroom
        (lib.concatStrings (builtins.genList (_: "x") 108)));
    };
    expected = {
      linuxUnixPathMaxBytes = 107;
      shortIdLength = 20;
      vectorLength = 20;
      vectorContainsNul = false;
      singleIdHeadroom = 87;
      maxPathHeadroom = 0;
      overlongAccepted = false;
    };
  };

  "v2-identity/literal-nul-escape-is-not-a-nul-byte" = {
    expr = identity.unixPathHeadroom "foo\\u0000";
    expected = 98;
  };
}
