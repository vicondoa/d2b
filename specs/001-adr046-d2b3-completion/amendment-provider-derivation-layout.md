# Amendment request: Provider derivation artifact layout

| Field | Value |
| --- | --- |
| Scope | The required derivation outputs of a Provider artifact: output name, file paths, executable set, digest preimages, signature anchoring, and the conformance scenarios that check them |
| Raised under | The W5 audit, recorded in `implementation-debt.md` sections 12.1, 12.2, 12.3, 14.8, and 19.7 |
| Deciding record | [ADR 0050](../../docs/adr/0050-provider-derivation-artifact-layout.md), currently **Proposed** |
| Affected member specs | `ADR-046-resources-zone-control` (sections 4.3.1, 4.9 new, 14.10, 15.8, 17); `ADR-046-provider-model-and-packaging` (Package catalog); `ADR-046-nix-configuration` (Validation); `ADR-046-decision-register` (D101 domain tags); provider dossiers `system-core` and `transport-azure-relay` |
| Affected manifests | `ADR-046-work-items.json`, `ADR-046-implementation-graph.json`, `ADR-046-implementation-graph.md`, `ADR-046-spec-set.json` - all four are **generated**, see section 8 |
| Unblocks | `ADR046-zone-control-015` (T174), and transitively `ADR046-zone-control-016` (T212) and `ADR046-zone-control-021` (T213) |
| Status | Drafted. **Not applied.** ADR 0050 is Proposed; the edits below land once it is Accepted |

## 0. Why the text is drafted here rather than applied

`docs/specs/` is normative and its work-item manifests are drift-gated. Landing a
normative amendment against a Proposed ADR would put the spec set ahead of the
decision it cites, which is the failure mode the register exists to catch. The
exact replacement text is therefore written out here, verbatim and applyable, and
the edit lands in the change that flips ADR 0050 to Accepted.

Every edit below is a replacement or an insertion with its anchor quoted, so the
implementer applying it does not have to infer placement.

## 1. `ADR-046-resources-zone-control` section 4.3.1: correct the mislocated map

### Replace this row

> | `executableDigests` | map[name]sha256 | One entry per built binary; validated at build |

### With

> | `executableDigest` | sha256 | Digest over the signed manifest's `package.executableDigests` object; see section 4.9.4. The per-binary map itself is a manifest field, not a catalog field |

**Reason.** Section 4.3.3 defines `binaryRef` three times as "Key in
`package.executableDigests`", which is the manifest's `package` object. The
shipped catalog shape `nixos-modules/generated/provider-catalog-shape.nix`
carries a singular `executableDigest`, and the shipped contract
`packages/d2b-contracts/src/v3/provider.rs` carries
`ArtifactDigestSet::executable`, documented "The component executable set
digest". Generated code and contract agree; the 4.3.1 table is the outlier.
Existing code is canon.

### Append to the paragraph that follows the table

> The exact on-disk shape of a Provider derivation, the relative path of each
> required file, and the preimage of every digest in this table are fixed by
> section 4.9.

## 2. `ADR-046-resources-zone-control`: insert a new section 4.9

Insert immediately after section 4.8.5 ("Example skeleton") and before the
`---` that precedes section 5.

---

### 4.9 Provider derivation artifact layout (normative)

Section 4.8 fixes the shape of a Provider **crate**. This section fixes the
shape of the **derivation** that crate builds, which is what
`d2b.artifacts.<id>.package` names and what the resource compiler validates at
Phase 2. The two are independent: a crate layout is a repository fact, a
derivation layout is a build output fact.

#### 4.9.1 Exactly one Nix output

A `type = "provider"` artifact's package MUST have exactly one output. The
Phase 1 assertion is `builtins.length package.all == 1`, evaluated in
`nixos-modules/provider-catalog.nix`. The `all` attribute is used rather than
`outputs` because a single-output derivation may carry no `outputs` attribute at
all, while `all` is present on every derivation.

The rule exists because `provider-catalog.nix` records
`storePath = "${artifact.package}"`, and for a multi-output derivation that
string is the **first** output rather than `out`. A split Provider package would
therefore pin one output while its manifest sat in another.

The eval failure names the artifact ID, the observed output names, and the
remediation: merge the split outputs, or package the second concern as its own
artifact.

Every path below is relative to the single store path
`"${artifact.package}"` yields, written `<out>`.

#### 4.9.2 Required paths

```text
<out>/
|-- bin/
|   `-- <name>                                  one regular file per executable
`-- share/d2b/provider/
    |-- provider-manifest.json                  the signed manifest
    |-- provider-manifest.json.sig              detached signature over its octets
    `-- config-schema.json                      the root config JSON Schema
```

| Path | Required contents | Absent or wrong-typed |
| --- | --- | --- |
| `<out>/share/d2b/provider/provider-manifest.json` | The signed Provider manifest of section 4.3, serialized as `d2b-cjson/v1` with no trailing newline | build failure `nix-build-required-outputs-missing` |
| `<out>/share/d2b/provider/provider-manifest.json.sig` | Exactly 64 octets: a detached Ed25519 signature over the manifest file's raw octets. No framing, no base64, no trailing newline | build failure `nix-build-required-outputs-missing` if absent, `nix-build-manifest-signature-invalid` if the length is wrong |
| `<out>/share/d2b/provider/config-schema.json` | The root JSON Schema that `spec.config` is validated against (section 4.3.2), serialized as `d2b-cjson/v1` with no trailing newline | build failure `nix-build-required-outputs-missing` |
| `<out>/bin/<name>` | One regular file per built component executable. Present if and only if `package.executableDigests` is non-empty | build failure `nix-build-executable-set-mismatch` or `nix-build-executable-not-regular-file` |

The three names under `share/d2b/provider/` are constants. They embed no Provider
identity, version, or artifact ID, because the compiler must locate the manifest
before it is entitled to believe anything the manifest declares about identity.

Both directories are closed. `bin/` holds exactly the executable set and no
subdirectory; `share/d2b/provider/` holds exactly those three files. The
remainder of the output is unconstrained and unpinned, and the resource compiler
MUST NOT read any path outside these two directories.

A Provider that ships no executable of its own declares an empty
`package.executableDigests` and ships no `<out>/bin`. `Provider/system-core` is
such a Provider: its handlers link into the `d2b-core-controller` binary, which
is built by a separate derivation and is not a Provider artifact. A manifest
whose component descriptors carry any `binaryRef` MUST declare a non-empty
`package.executableDigests`.

The derivation is world-readable in the Nix store. No file in this layout may
carry a credential, token, key, host path, or PID. Secrets remain
`Credential/<name>` references resolved at runtime.

#### 4.9.3 Locating the executable set

The resource compiler computes three sets and requires them consistent:

1. the directory entries of `<out>/bin`;
2. the key set of the signed manifest's `package.executableDigests`;
3. the set of `binaryRef` values across the manifest's controller, service, and
   worker descriptors.

(1) and (2) MUST be equal. (3) MUST be a subset of (2), because a Provider may
ship a binary that no component descriptor launches directly.

Each `<out>/bin/<name>` MUST be a regular file. A symlink is refused because it
can resolve outside the pinned output, placing digested bytes outside the
package digest. A directory is refused. Each `<name>` matches `^[a-z][a-z0-9-]*$`
and is at most 64 bytes.

A Process created for a Provider component resolves its program as
`<out>/bin/<binaryRef>` and by no other means: no `PATH` lookup, no
manifest-supplied absolute path, no path relative to a working directory.

#### 4.9.4 Digest preimages

Every value renders `sha256:<64 lowercase hex>`.

| Value | Carried by | Preimage |
| --- | --- | --- |
| `package.executableDigests[<name>]` | signed manifest | SHA-256 over the raw octets of `<out>/bin/<name>` |
| `executableDigest` | artifact catalog | `canonical_digest("d2b:v3:provider-executable-set", C)`, where `C` is the `d2b-cjson/v1` serialization of the manifest's `package.executableDigests` object |
| `manifestDigest` | artifact catalog | SHA-256 over the raw octets of `<out>/share/d2b/provider/provider-manifest.json` |
| `configSchemaDigest` | artifact catalog and signed manifest | SHA-256 over the raw octets of `<out>/share/d2b/provider/config-schema.json` |
| `digest` | artifact catalog | SHA-256 of the NAR serialization of `<out>`, as `nix hash path --type sha256 --base16` renders it |

Executable digests carry no domain tag: an ELF image is not canonical JSON and
there is nothing to canonicalize. The executable set digest is domain-separated
under D101 using the existing `canonical_digest` helper,
`SHA-256(domain_tag || 0x00 || canonical_bytes)`.

`provider-manifest.json` and `config-schema.json` MUST already be their own
`d2b-cjson/v1` canonical bytes. The compiler parses each file, re-serializes it
canonically, and refuses unless the result is byte-identical to the file it read.
Without that rule a digest recorded over one byte-spelling would appear to attest
another with identical parsed content.

`closureDigest` and `closureSize` are unchanged and remain owned by
`ADR046-nix-022`.

#### 4.9.5 Three-source agreement

Three parties describe the same artifact and none is trusted alone:

- the **operator**, through the authored `d2b.artifacts.<id>.catalog` digests
  that `nixos-modules/provider-catalog.nix` already requires present and
  well-formed;
- the **publisher**, through the signed manifest's `package.executableDigests`
  and `package.configSchemaDigest`;
- **Nix**, through the values the resource compiler recomputes from `<out>`.

The compiler computes the Nix side itself and never copies a value from the
manifest or the catalog into the side it is checking. Admission requires pairwise
equality on the manifest digest, the config schema digest, the executable set,
and the package content digest. A disagreement fails the build naming the two
disagreeing sources and the value.

#### 4.9.6 Signature anchoring

`provider-manifest.json.sig` is verified before any other file in the derivation
is read. The public key is resolved from the built-in `d2b-official` root or from
`d2b.zones.<zone>.trustedPublishers.<publisher>.signingKey`, a PEM
SubjectPublicKeyInfo carrying an Ed25519 key, selected by the catalog entry's
`publisher` and `signatureId`. An unresolvable `signatureId`, an unregistered
publisher, a wrong-length signature file, or a failed verification is a hard
build failure; none of them is a warning.

The signature covers the manifest only. Every other file in the derivation is
reached through a digest the verified manifest declares, so the chain is: store
path immutability pins the bytes, the operator pin and the Nix recomputation pin
the manifest, the signature binds the manifest to a publisher, and the manifest
binds every remaining file.

The manifest cannot carry its own digest or the package content digest, because
both are computed over documents containing it. Those two are the Nix and
operator sides of 4.9.5.

Trust material on the host is public verification key material only. Publisher
signing keys never enter `d2b.artifacts`, the host closure, or any host-side
activation artifact.

---

## 3. `ADR-046-resources-zone-control` section 14.10: replace one row, add one

### Phase 1 table: insert after the last Provider row

> | Provider artifact package has exactly one Nix output (`builtins.length package.all == 1`) - Provider only | Nix `assert` in `provider-catalog.nix` | eval error naming the artifact ID and the observed output names |

### Phase 2 table: replace this row

> | Artifact catalog entry has required derivation outputs (manifest, config
> schema, executable) - Provider only | Resource compiler | build failure |

### With these six rows

> | Required derivation paths present per section 4.9.2 (`share/d2b/provider/provider-manifest.json`, `provider-manifest.json.sig`, `config-schema.json`) - Provider only | Resource compiler | build failure naming the absent relative path |
> | `provider-manifest.json` and `config-schema.json` octets equal their own `d2b-cjson/v1` canonical bytes - Provider only | Resource compiler | build failure naming the non-canonical file |
> | `bin/` entry set equals the signed manifest's `package.executableDigests` key set; each entry is a regular file - Provider only | Resource compiler | build failure naming the symmetric difference or the non-regular entry |
> | Every component `binaryRef` is a key of `package.executableDigests` - Provider only | Resource compiler | build failure naming the component and the ref |
> | Each `bin/<name>` SHA-256 equals its `package.executableDigests` value - Provider only | Resource compiler | build failure naming the binary |
> | Operator-authored catalog digests, manifest-declared digests, and compiler-recomputed digests agree pairwise (section 4.9.5) - Provider only | Resource compiler | build failure naming the two disagreeing sources and the value |

### Phase 2 table: the signature row keeps its wording and gains a scenario

The existing row

> | Artifact manifest signature chain valid against installed trust store - Provider only | Resource compiler | build failure |

is unchanged in wording. Its conformance scenario,
`nix-build-manifest-signature-invalid`, is added in section 4 below; it had none.

## 4. `ADR-046-resources-zone-control` section 15.8: add the missing scenarios

### Phase 1 - Nix eval tests: append

> | `nix-eval-provider-multiple-outputs` | A `type = "provider"` artifact whose package declares more than one Nix output fails eval naming the artifact ID and the output names |

### Phase 2 - Build tests: insert after `nix-build-manifest-digest-mismatch`

> | `nix-build-required-outputs-missing` | A Provider derivation missing any of `share/d2b/provider/provider-manifest.json`, `provider-manifest.json.sig`, or `config-schema.json` fails build naming the absent relative path |
> | `nix-build-manifest-signature-invalid` | A signature file that is not exactly 64 octets, or that does not verify against the resolved publisher key, fails build; an unresolvable `signatureId` or unregistered publisher fails the same way |
> | `nix-build-manifest-not-canonical` | A manifest or config schema whose octets are not their own `d2b-cjson/v1` canonical bytes fails build naming the file |
> | `nix-build-executable-set-mismatch` | `package.executableDigests` keys unequal to the `bin/` entry set fails build naming the symmetric difference |
> | `nix-build-executable-not-regular-file` | A `bin/` entry that is a symlink or a directory fails build naming the entry |
> | `nix-build-binary-ref-unresolved` | A component descriptor `binaryRef` absent from `package.executableDigests` fails build naming the component and the ref |
> | `nix-build-executable-digest-mismatch` | A `bin/` file whose SHA-256 differs from its `package.executableDigests` value fails build naming the binary |
> | `nix-build-catalog-manifest-disagreement` | Operator-authored catalog digests, manifest-declared digests, and compiler-recomputed digests disagreeing on any pinned value fails build naming the two disagreeing sources |

## 5. `ADR-046-resources-zone-control` section 17: correct two Validation fields

Both cite scenario identifiers that do not exist in section 15.8. Verified by
enumerating the whole table: its Phase 2 entries are
`nix-build-artifact-id-missing-from-catalog`,
`nix-build-artifact-wrong-type-rejected`, `nix-build-duplicate-artifact-id`,
`nix-build-artifact-store-path-absent-from-bundle`,
`nix-build-artifact-store-path-absent-from-config`,
`nix-build-config-schema-failure`, `nix-build-schema-digest-mismatch`,
`nix-build-manifest-digest-mismatch`, `nix-build-resourcetype-collision`,
`nix-build-bundle-sorted`, `nix-build-content-hash-stable`,
`nix-build-artifact-catalog-digest-anchored`,
`nix-build-credential-ref-survives-build`,
`nix-build-inline-secret-lint-warning`, and
`nix-build-inline-secret-strict-failure`.

### 5.1 `ADR046-zone-control-015`

| Cited identifier | Replacement | Reason |
| --- | --- | --- |
| `nix-build-bundle-digest-stable` | `nix-build-content-hash-stable` | The bundle's stable-identity field is `contentHash`; section 14.9 states there is no separate `bundleDigest` |
| `nix-build-per-resource-digest-correct` | `nix-build-artifact-catalog-digest-anchored` | Section 14.9 states outright that there is no per-resource `digest` member, so the cited name asks for a check the bundle contract forbids. The fifteenth Phase 2 entry this item owns, and omitted, is the catalog-digest anchor |

Replace the Validation cell with:

> | Validation | All Phase 2 build tests in §15.8 (`nix-build-artifact-id-missing-from-catalog`, `nix-build-artifact-wrong-type-rejected`, `nix-build-duplicate-artifact-id`, `nix-build-artifact-store-path-absent-from-bundle`, `nix-build-artifact-store-path-absent-from-config`, `nix-build-config-schema-failure`, `nix-build-schema-digest-mismatch`, `nix-build-manifest-digest-mismatch`, `nix-build-required-outputs-missing`, `nix-build-manifest-signature-invalid`, `nix-build-manifest-not-canonical`, `nix-build-executable-set-mismatch`, `nix-build-executable-not-regular-file`, `nix-build-binary-ref-unresolved`, `nix-build-executable-digest-mismatch`, `nix-build-catalog-manifest-disagreement`, `nix-build-resourcetype-collision`, `nix-build-bundle-sorted`, `nix-build-content-hash-stable`, `nix-build-artifact-catalog-digest-anchored`, `nix-build-credential-ref-survives-build`, `nix-build-inline-secret-lint-warning`, `nix-build-inline-secret-strict-failure`) and the Phase 1 eval test `nix-eval-provider-multiple-outputs` |

Also append to its Detailed design cell, after "extract and hash manifest and
config schema files":

> at the fixed relative paths section 4.9.2 fixes, refusing a derivation with
> more than one Nix output at Phase 1 and a derivation missing any required path
> at Phase 2

### 5.2 `ADR046-zone-control-016`

| Cited identifier | Replacement | Reason |
| --- | --- | --- |
| `nix-runtime-bundledigest-integrity` | `nix-runtime-content-hash-integrity` | `contentHash` naming, per section 14.9 |
| `nix-runtime-generation-monotone` | `nix-runtime-same-content-hash-noop` | The generation ordinal is assigned and committed by `ADR046-routing-013`, which this item's own Detailed design already states. The runtime property `016` owns is that an identical `contentHash` is a no-op rather than a re-activation |
| `nix-runtime-zonename-mismatch-rejected` | `nix-runtime-zone-mismatch-rejected` | Spelling only |

Replace the Validation cell with:

> | Validation | All Phase 3 runtime and cleanup tests in §15.8 (`nix-runtime-content-hash-integrity`, `nix-runtime-same-content-hash-noop`, `nix-runtime-zoneuid-mismatch-rejected`, `nix-runtime-zone-mismatch-rejected`, `nix-runtime-activation-nonblocking`, `nix-runtime-provider-config-invalid-continues`, all `cleanup-*` and `rollback-*` tests) |

### 5.3 `ADR046-zone-control-021`

No correction. Its cited scenarios all exist. It is named here because the W5
audit named it, and because its blocking status was inherited through `016` and
lifts with this amendment.

## 6. Provider dossier corrections

### 6.1 `docs/specs/providers/ADR-046-provider-transport-azure-relay.md`

Replace the artifact resolution block:

```text
packages/d2b-provider-transport-azure-relay/
  -> d2b-transport-azure-relay-listener   (system binary)
  -> d2b-transport-azure-relay-sender     (system binary)
  -> provider-manifest.json               (signed)
  -> transport-settings.schema.json       (settings schema; committed separately
                                           under docs/reference/schemas/v3/providers/)
```

with:

```text
<out>/
  bin/d2b-transport-azure-relay-listener         (system binary)
  bin/d2b-transport-azure-relay-sender           (system binary)
  share/d2b/provider/provider-manifest.json      (signed)
  share/d2b/provider/provider-manifest.json.sig  (detached Ed25519 signature)
  share/d2b/provider/config-schema.json          (root config schema; the
                                                  reviewable copy is committed
                                                  under docs/reference/schemas/
                                                  v3/providers/ and kept equal by
                                                  make test-drift)
```

The committed copy under `docs/reference/schemas/v3/providers/` remains the
review surface and stays bound by `xtask gen-provider-transport-schemas &&
git diff --exit-code`. The derivation copy at
`share/d2b/provider/config-schema.json` is the copy the resource compiler hashes.
For a third-party Provider only the derivation copy exists.

### 6.2 `docs/specs/providers/ADR-046-provider-system-core.md`

Replace the "The crate produces" table row

> | `provider-system-core-manifest.json` | Compiled provider manifest installed into the private artifact catalog |

with

> | `share/d2b/provider/provider-manifest.json` | Compiled provider manifest at the fixed path of section 4.9.2; its store path is recorded in the private artifact catalog |

and add a row recording the empty executable set:

> | (no `bin/`) | `system-core` builds no component executable of its own, so its manifest declares an empty `package.executableDigests` and the derivation ships no `bin/` directory. `libsystem_core.rlib` is an internal build product linked by the separate `d2b-core-controller` derivation; it is not a Provider artifact and is not pinned by any catalog digest |

Update the prose two paragraphs below, which repeats
`provider-system-core-manifest.json`, to the same fixed path.

## 7. Decision register and packaging-spec touches

### 7.1 `ADR-046-decision-register` D101

Register the new domain tag alongside the nine in use
(`d2b:v3:artifact-catalog`, `d2b:v3:credential-lease-handle`,
`d2b:v3:credential-source-version`, `d2b:v3:resource-bundle`,
`d2b:v3:resource-envelope`, `d2b:v3:resource-schema`, `d2b:v3:resource-spec`,
`d2b:v3:resource-status`, `d2b:v3:schema`):

> `d2b:v3:provider-executable-set` - the digest of a Provider's per-binary
> executable digest map, computed over the `d2b-cjson/v1` serialization of the
> signed manifest's `package.executableDigests` object. It is the value the
> artifact catalog records as `executableDigest`.

### 7.2 `ADR-046-provider-model-and-packaging`, "Crate/package boundary"

The bullet `- has one Nix package/conformance output;` is now checkable and
should cite where: append `(exactly one Nix output; layout fixed by
ADR-046-resources-zone-control section 4.9)`.

### 7.3 `ADR-046-nix-configuration`, "Validation" table

Add a row:

> | Provider artifact package has exactly one Nix output | Eval |

## 8. Applying this amendment: the manifests are generated

`docs/specs/ADR-046-work-items.json`, `ADR-046-implementation-graph.json`,
`ADR-046-implementation-graph.md`, and `ADR-046-spec-set.json` are **generated**
from the markdown by `xtask spec-registry` and `xtask implementation-graph`, and
are drift-gated by `tests/unit/gates/drift-check.sh`. They MUST NOT be
hand-edited. The applying change edits the markdown, then runs:

```bash
cargo run --manifest-path packages/Cargo.toml -p xtask -- spec-registry
cargo run --manifest-path packages/Cargo.toml -p xtask -- implementation-graph
```

and commits the regenerated JSON in the same commit. `make test-drift` is the
gate that proves it. `gen_spec_set.rs` also asserts a fixed corpus size
(`EXPECTED_MEMBERS = 55`, `EXPECTED_WORK_ITEMS = 545`); this amendment adds no
member and no work item, so both counts are unchanged and a change in either is
a signal that the edit did something unintended.

## 9. Register rows this amendment closes

| `implementation-debt.md` row | Disposition |
| --- | --- |
| Required derivation outputs have no path, filename, output name, or layout (12.2) | Closed by ADR 0050 items 1, 2, 4 and section 2 above |
| Required-outputs row has no conformance scenario in the section 15.8 Phase 2 table (12.3) | Closed by section 4 above |
| Output cardinality not checkable: no Provider crate has a package output (12.2) | Closed by ADR 0050 item 1: the cardinality is now an eval assertion on the artifact entry, which exists whether or not any Provider crate ships a package yet |
| `d2b.artifacts.<id>.package` typed `types.package` already enforces the cardinality at the one entry point (12.2, "inference, needs confirm or reject") | **Reject.** `types.package` pins one derivation per artifact ID; it does not pin one output per derivation, and `"${package}"` selecting the first output is exactly the case it misses. The inference is superseded by the explicit `all` assertion |
| `ADR046-zone-control-015` stays blocked pending an amendment (19.7) | Closed on acceptance; `016` and `021` unblock with it |
| Catalog names component and descriptor digests; contract names exported schema and service digests (12.4) | **Partly narrowed, not closed.** The executable digest was never part of the dispute: the catalog's singular value and the manifest's map are different objects, and section 2 states the derivation rule. The component/descriptor versus schema/service pair remains open and remains `ADR046-provider-002`'s |

## 10. Drift observed while drafting, recorded not fixed

Not in scope for this amendment; recorded so it is not lost.

| Fact | Where |
| --- | --- |
| `artifactId` maximum length is three different values: 128 characters in section 4.3.1, `maxArtifactIdLength = 64` in `nixos-modules/provider-catalog.nix`, and `MAX_ARTIFACT_ID_BYTES: usize = 63` in `packages/d2b-contracts/src/v3/provider.rs`. Two of the three are shipped code and they disagree with each other | 4.3.1 vs `provider-catalog.nix` vs `provider.rs` |
| `implementation-debt.md` 12.2 records "no Provider crate carries a `.nix` file". That is no longer true: six exist across four crates - `d2b-provider-network-local/nix/{default,artifacts,net-vm}.nix`, which are NixOS modules registering artifacts, and one `integration/*.nix` scenario declaration under each of `credential-entra`, `credential-managed-identity`, and `credential-secret-service`. None is a package derivation, so 12.2's conclusion survives, but its stated evidence does not | `implementation-debt.md` 12.2 |
| The root config schema is spelled three ways across the set: `config` (D075 and shipped `ProviderSpec::config`), `settingsSchemaDigest` in the `provider-catalog.json` example, and `configDigest` in the generated catalog shape. D075 and shipped code agree on `config` | D075, `ADR-046-nix-configuration`, `provider-catalog-shape.nix` |
| `SPIKE-05`, which would have exercised exactly this layout before it was specified, is recorded "Specified - not yet executed" and `proofs/provider-packaging-spike/` does not exist | `ADR-046-feasibility-and-spikes` |
