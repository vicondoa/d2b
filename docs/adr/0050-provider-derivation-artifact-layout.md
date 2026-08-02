# ADR 0050: Provider derivation artifact layout

- Status: Proposed
- Date: 2026-08-02
- Related: [ADR 0046](0046-d2b-3-provider-control-plane.md) (d2b 3.0 Provider
  control plane) and its decisions D012, D059, D075, D101, and D120 in
  [`docs/specs/ADR-046-decision-register.md`](../specs/ADR-046-decision-register.md);
  [ADR 0015](0015-daemon-only-clean-break.md) for the daemon-only control plane
  the resource plane sits inside; [ADR 0032](0032-d2b-v2-constellation-control-plane.md)
  and [ADR 0043](0043-realm-native-control-plane.md) for the rule that no host
  process holds realm credentials
- Scope: the on-disk shape of a Provider Nix derivation, the digest preimages
  that pin it, and the Phase 1/Phase 2 checks that refuse a derivation which
  does not match. Consumed by `ADR046-zone-control-015` (the resource
  compiler), `ADR046-zone-control-016`, and `ADR046-zone-control-021`.

## Context

`docs/specs/ADR-046-resources-zone-control.md` section 14.10, "Phase 2 - Nix
build", carries this row:

> Artifact catalog entry has required derivation outputs (manifest, config
> schema, executable) - Provider only | Resource compiler | build failure

The row names three required outputs and specifies no path, no filename, no Nix
output name, and no directory layout for any of them. The W5 audit recorded that
in `specs/001-adr046-d2b3-completion/implementation-debt.md` sections 12.1
through 12.3, and section 19.7 ruled `ADR046-zone-control-015` blocked rather
than delivered on invented facts. That blocking status propagates:
`ADR046-zone-control-016` names `015` as a prerequisite and
`ADR046-zone-control-021` depends on `016`.

The audit also recorded that the required-outputs row has no conformance
scenario anywhere in the section 15.8 Phase 2 table, so an implementation that
omitted the check entirely would satisfy the work item's own stated validation.

This ADR closes both holes. It is deliberately narrow: it fixes the shape of one
derivation and the digests that pin it, and it renames no catalog field.

### What the tree already fixes, measured rather than assumed

**Every d2b package is single-output.** `flake.nix` builds every Rust artifact
through `pkgs.rustPlatform.buildRustPackage` with no `outputs` list, installing
into `$out/bin/<name>`. There is no multi-output derivation anywhere in the
repository.

**`d2b.artifacts.<id>.package` is coerced with `"${...}"`.**
`nixos-modules/provider-catalog.nix` computes `storePath = "${artifact.package}"`.
For a multi-output derivation that string is the **first** output, not `out`:

```text
$ nix eval --impure --expr '... { coerced = "${pkgs.openssl}"; outOutput = "${pkgs.openssl.out}"; }'
{"coerced":"/nix/store/zyrxhd7nwmkcs11m144jagxcmddw2i41-openssl-3.6.2-bin",
 "outOutput":"/nix/store/y18pnbvfarnilsmgayswvi1khaw9wbsc-openssl-3.6.2",
 "outputName":"bin","same":false}
```

A Provider that split `outputs = [ "bin" "out" ]` would therefore have the
catalog pin `$bin` while its manifest sat in `$out`, unreachable from the pinned
path and outside the digest the catalog records for it.

**A single-output derivation is detectable at eval time, and `outputs` is not
the attribute to test.** A raw `builtins.derivation` with one output carries no
`outputs` attribute at all, while `all` is present on every derivation:

```text
{"singleHasOutputs":false,"singleAllLen":1,
 "multiHasOutputs":true,"multiOutputs":["out","lib"],"multiAllLen":2}
$ # and on real nixpkgs derivations:
{"helloAllLen":1,"helloOutputs":["out"],
 "opensslAllLen":6,"opensslOutputs":["bin","dev","out","man","doc","debug"]}
```

So `builtins.length package.all == 1` is a pure, total, eval-time single-output
test that holds for `stdenv.mkDerivation`, `buildRustPackage`, and the bare
primop alike. `package.outputs` is not, because its absence means "one output".

**Two file names are already attested, and they disagree.**
`docs/specs/providers/ADR-046-provider-transport-azure-relay.md` resolves an
artifact entry to two system binaries plus `provider-manifest.json (signed)` and
a settings schema. `docs/specs/providers/ADR-046-provider-system-core.md` names
`provider-system-core-manifest.json` instead. One of the two spellings has to
lose; an identity-parameterized name also forces the compiler to learn the
Provider identity before it can find the file that declares the identity.

**The per-binary digest map is a manifest field, not a catalog field.** Section
4.3.3 defines `binaryRef` three times as "Key in `package.executableDigests`" -
the manifest's `package` object. The shipped catalog shape
`nixos-modules/generated/provider-catalog-shape.nix` carries a singular
`executableDigest`, and the shipped contract
`packages/d2b-contracts/src/v3/provider.rs` carries
`ArtifactDigestSet::executable`, documented as "The component executable set
digest". Generated code and contract agree with each other; section 4.3.1's
table is what mislocated the map into the catalog. Existing code is canon.

**No Provider crate has a package output today.** Fourteen `packages/d2b-provider-*`
crates exist and six `.nix` files sit across four of them: three NixOS modules
under `d2b-provider-network-local/nix/` that register artifacts into
`d2b.artifacts`, and three `integration/*.nix` scenario declarations under the
credential Providers. None is a package derivation, and `flake.nix` exposes no
Provider package attribute. `SPIKE-05`, which
would have exercised exactly this layout, is recorded "Specified - not yet
executed" and `proofs/provider-packaging-spike/` does not exist. There is
therefore no in-tree precedent to defer to, which is why the layout has to be
decided rather than discovered.

### Non-negotiable constraints

- The Provider derivation lives in `/nix/store` and is world-readable. Nothing
  in the required layout may be sensitive.
- Trust material on the host is public verification key material only. Publisher
  signing keys never enter `d2b.artifacts`, the host closure, or any host-side
  activation artifact, and this ADR creates no host credential ownership
  (ADR 0032, ADR 0043).
- The daemon-only end-state holds. Nothing here declares a unit; the resource
  compiler is a build-time program and the launcher is `d2bd`'s DAG executor
  (ADR 0015).

## Decision

### 1. A Provider derivation has exactly one Nix output

The output is the default `out`. `nixos-modules/provider-catalog.nix` asserts,
for every `d2b.artifacts.<id>` entry with `type = "provider"`:

```nix
builtins.length artifact.package.all == 1
```

`all` is used rather than `outputs` because a single-output derivation may
carry no `outputs` attribute, as measured above. The failure is an eval error
naming the artifact ID, the observed output names, and the remediation: "a
Provider derivation must have exactly one output; merge the split outputs or
package the Provider separately." This is Phase 1, so it fails before any build
starts.

Every path this ADR fixes is relative to the single store path that
`"${artifact.package}"` yields. That makes "the required outputs are present"
computable from the one value the catalog already records.

### 2. The required paths are fixed, and identity-independent

```text
<out>/
|-- bin/
|   `-- <name>                                  one regular file per executable
`-- share/d2b/provider/
    |-- provider-manifest.json                  the signed manifest
    |-- provider-manifest.json.sig              detached signature over its octets
    `-- config-schema.json                      the root config JSON Schema
```

The three names under `share/d2b/provider/` are constants. They do not embed the
Provider identity, the version, or the artifact ID, because the compiler must be
able to locate the manifest before it is entitled to believe anything the
manifest says about identity. `share/` is correct rather than `lib/` or
`libexec/` because both files are architecture-independent data.

`bin/` exists if and only if the manifest's `package.executableDigests` is
non-empty. A Provider that ships no executable of its own - `system-core`, whose
handlers link into the `d2b-core-controller` binary - declares an empty
`executableDigests` and ships no `bin/`. A manifest whose components carry any
`binaryRef` MUST declare a non-empty `executableDigests`.

Both directories are closed. `bin/` contains exactly the executable set and no
subdirectory. `share/d2b/provider/` contains exactly those three files. The rest
of the output is unconstrained and unpinned, and the resource compiler MUST NOT
read any path outside these two directories.

### 3. The executable set is located by enumerating `bin/`, never by trusting a claim

For a Provider artifact, the compiler computes three sets and requires them
equal:

1. the directory entries of `<out>/bin`;
2. the key set of the signed manifest's `package.executableDigests`;
3. the set of `binaryRef` values appearing in the manifest's component
   descriptors, which must be a subset of (2) rather than equal to it, because a
   Provider may ship a binary that no component descriptor launches directly.

Each `<out>/bin/<name>` MUST be a regular file. A symlink is refused, because a
symlink can resolve outside the pinned output and its digest would then cover
bytes the package digest does not. A directory is refused. Each `<name>` matches
`^[a-z][a-z0-9-]*$` and is at most 64 bytes.

A mismatch fails the build naming the symmetric difference and the side each
name came from.

### 4. Digest preimages, stated exactly

Every value below renders as `sha256:<64 lowercase hex>`, which is what
`ArtifactDigest::parse` in `packages/d2b-contracts/src/v3/provider.rs` already
admits and what `provider-catalog.nix` already asserts at eval time.

| Value | Preimage |
| --- | --- |
| `package.executableDigests[<name>]` (manifest) | SHA-256 over the raw octets of `<out>/bin/<name>` |
| executable set digest (catalog) | `canonical_digest("d2b:v3:provider-executable-set", C)` where `C` is the `d2b-cjson/v1` serialization of the `executableDigests` object |
| `manifestDigest` | SHA-256 over the raw octets of `<out>/share/d2b/provider/provider-manifest.json` |
| `configSchemaDigest` | SHA-256 over the raw octets of `<out>/share/d2b/provider/config-schema.json` |
| package content digest | SHA-256 of the NAR serialization of the single output store path, as `nix hash path --type sha256 --base16` renders it |

Executable digests are raw file digests with no domain tag, because an ELF image
is not canonical JSON and there is nothing to canonicalize. The executable set
digest is domain-tagged because it is a JSON document, and it uses the shipped
`canonical_digest` helper unchanged: `SHA-256(domain_tag || 0x00 ||
canonical_bytes)`. `d2b:v3:provider-executable-set` is a new D101 domain tag and
registers alongside the nine already in use.

`provider-manifest.json` and `config-schema.json` MUST already be in
`d2b-cjson/v1` canonical form, with no trailing newline, so that their file
octets and their canonical bytes are the same string. The compiler parses each
file, re-serializes it canonically, and refuses unless the result is byte-
identical to the file it read. Without that rule a digest recorded over one
byte-spelling of a document would appear to attest a different byte-spelling
with identical parsed content, and the reviewer reading the file would not be
looking at the bytes the digest covers.

Closure-level values (`closureDigest`, `closureSize`) are unchanged and remain
owned by `ADR046-nix-022`.

### 5. Admission requires three independently derived sources to agree

Three parties describe the same artifact and none of them is trusted to describe
it alone:

- the **operator**, through the authored `d2b.artifacts.<id>.catalog` digests
  that `provider-catalog.nix` already requires present and well-formed;
- the **publisher**, through the signed manifest's `package.executableDigests`
  and `package.configSchemaDigest`;
- **Nix**, through what the resource compiler recomputes from the store path at
  Phase 2.

The compiler computes the Nix side itself and never copies a value from the
manifest or the catalog into the side it is checking. Admission requires pairwise
equality on the manifest digest, the config schema digest, the executable set,
and the package content digest. Any disagreement is a build failure naming which
two sources disagreed and on which value.

This is what makes exact-digest selection mean something. An operator pin that
was merely echoed from the artifact it is supposed to select would be decoration.

### 6. One signature, over one file, and everything else is reached through it

`provider-manifest.json.sig` is a detached Ed25519 signature over the raw octets
of `provider-manifest.json`. The file contains exactly the 64 signature octets
with no framing, so a wrong-sized file is refused before any parse. The public
key is resolved from the built-in `d2b-official` root or from
`d2b.zones.<zone>.trustedPublishers.<publisher>.signingKey`, a PEM
SubjectPublicKeyInfo carrying an Ed25519 key, selected by the catalog entry's
`signatureId` and `publisher`. An unresolvable `signatureId`, an unknown
publisher, or a failed verification is a hard build failure, never a warning.

The signature covers the manifest only. It does not need to cover the binaries
or the schema, because the verified manifest carries their digests and every
other file in the derivation is reachable only through a digest that file
declares. The chain is: store path immutability pins the bytes, the operator pin
and the Nix recomputation pin the manifest, the signature binds the manifest to a
publisher, and the manifest binds every remaining file.

The manifest cannot carry its own digest or the package content digest, since
both are computed over documents that contain it. Those two are the Nix and
operator sides of item 5, and that asymmetry is deliberate rather than an
omission.

### 7. `<out>/bin/<binaryRef>` is the only program path the framework will launch

A Process created for a Provider component resolves its program as
`<out>/bin/<binaryRef>` where `<out>` is the pinned artifact store path, and by
no other means: no `PATH` lookup, no manifest-supplied absolute path, no
argv[0] indirection, no path relative to a working directory.

This is what bounds the residual exposure of leaving the rest of the output
unpinned. An unpinned helper elsewhere in the closure is not framework-launchable;
it can only be executed by an already-launched pinned binary, which is inside the
trust boundary and inside its sandbox profile. That is the same posture the
framework already takes for every runner the broker spawns.

### 8. Nothing in the required layout may be sensitive

The derivation is world-readable in `/nix/store`. The manifest and the config
schema carry no credential, no token, no key, no host path, and no PID. Secrets
remain `Credential/<name>` refs resolved at runtime, exactly as section 4.3.2
already requires of `spec.config`. The trust root material the compiler reads is
public verification keys. No private key and no realm credential enters
`d2b.artifacts`, the host bundle, or any host-side activation artifact.

### 9. The checks and their conformance scenarios

The missing scenarios are added to section 15.8. Each is named so
`ADR046-zone-control-015`'s validation field can cite it.

| Scenario | Phase | Assertion |
| --- | --- | --- |
| `nix-eval-provider-multiple-outputs` | 1 | A `type = "provider"` artifact whose package has more than one output fails eval naming the artifact ID and the output names |
| `nix-build-required-outputs-missing` | 2 | A Provider derivation missing any of `share/d2b/provider/provider-manifest.json`, `provider-manifest.json.sig`, or `config-schema.json` fails build naming the absent relative path |
| `nix-build-manifest-signature-invalid` | 2 | A `.sig` that is not 64 octets, or that does not verify against the resolved publisher key, fails build; an unresolvable `signatureId` fails the same way |
| `nix-build-manifest-not-canonical` | 2 | A manifest or config schema whose octets are not their own `d2b-cjson/v1` canonical bytes fails build naming the file |
| `nix-build-executable-set-mismatch` | 2 | `executableDigests` keys unequal to the `bin/` entry set fails build naming the symmetric difference |
| `nix-build-executable-not-regular-file` | 2 | A `bin/` entry that is a symlink or a directory fails build naming the entry |
| `nix-build-binary-ref-unresolved` | 2 | A component `binaryRef` absent from `executableDigests` fails build naming the component and the ref |
| `nix-build-executable-digest-mismatch` | 2 | A `bin/` file whose SHA-256 differs from its `executableDigests` value fails build naming the binary |
| `nix-build-catalog-manifest-disagreement` | 2 | Operator-authored catalog digests, manifest-declared digests, and compiler-recomputed digests disagreeing on any pinned value fails build naming the two disagreeing sources |

### 10. The scenario names two work items cite are corrected

Five scenario identifiers cited as validation do not exist in section 15.8. They
are corrected rather than added, because in each case a real entry covers the
obligation and the cited name was a misremembering of it.

`ADR046-zone-control-015` (T174):

| Cited | Correct | Why |
| --- | --- | --- |
| `nix-build-bundle-digest-stable` | `nix-build-content-hash-stable` | The bundle's stable-identity field is `contentHash`; there is no `bundleDigest` (section 14.9) |
| `nix-build-per-resource-digest-correct` | `nix-build-artifact-catalog-digest-anchored` | Section 14.9 states outright that there is no per-resource `digest` member, so the cited name asks for a check the bundle contract forbids. The fifteenth Phase 2 entry the item actually owns, and omitted, is the catalog-digest anchor |

`ADR046-zone-control-016` (T212):

| Cited | Correct | Why |
| --- | --- | --- |
| `nix-runtime-bundledigest-integrity` | `nix-runtime-content-hash-integrity` | Same `contentHash` naming |
| `nix-runtime-generation-monotone` | `nix-runtime-same-content-hash-noop` | The generation ordinal is assigned and committed by `ADR046-routing-013`, not by `016`; the runtime property `016` owns is that an identical `contentHash` is a no-op rather than a re-activation |
| `nix-runtime-zonename-mismatch-rejected` | `nix-runtime-zone-mismatch-rejected` | Spelling only |

`ADR046-zone-control-021` (T213) cites no nonexistent scenario and needs no
correction; it is listed here because the W5 audit named it, and because it
inherits the unblocking through `016`.

### 11. What this ADR does not decide

The catalog-versus-contract digest reconciliation recorded in
implementation-debt section 12.4 stays open and stays owned by
`ADR046-provider-002`. This ADR fixes preimages for the digests section 4.3.1
already names and renames no catalog field, so
`CATALOG_FIELDS_WITHOUT_A_CONTRACT_FIELD` and
`CONTRACT_FIELDS_WITHOUT_A_CATALOG_FIELD` in
`packages/xtask/src/provider_packaging.rs` are untouched and the test that pins
them keeps failing on any unrecorded closure.

It does resolve one third of that dispute as a side effect, and the resolution is
that there was no dispute: the catalog's singular executable digest and the
manifest's `executableDigests` map are two different objects, and item 4 states
the rule deriving the first from the second. What remains disputed is the
component/descriptor versus schema/service pair, which is untouched here.

Publisher key custody, key rotation, and revocation distribution are out of
scope. This ADR consumes `signatureId`, `publisher`, `trustEpoch`, and
`revocationRef` as section 4.3.1 already defines them.

## Consequences

**A Provider package becomes a checkable object rather than an assertion.**
`ADR046-zone-control-015` can now write the Phase 2 required-outputs check
against fixed paths, and every one of the nine scenarios in item 9 is a
filesystem or digest comparison a machine evaluates. The blocking status recorded
in implementation-debt 19.7 lifts for `015`, and with it for `016` and `021`.

**Multi-output Provider packaging is foreclosed, and that costs something real.**
A Provider with large architecture-independent data, or one that wants a `dev`
output for a companion SDK, cannot express it as Nix outputs. It must ship a
second derivation and a second artifact entry, or accept a larger closure. The
alternative is worse: `"${package}"` silently selecting the first output is a
correctness bug that presents as a missing file at build time and as an
unpinned manifest at worst, and the eval assertion converts it into an error
that names itself.

**Byte-canonical JSON files are a real authoring burden.** A publisher cannot
hand-write `provider-manifest.json` and have it pass; it must be emitted by a
canonicalizing serializer, with no trailing newline, which will surprise anyone
who opens it in an editor that adds one. The toolkit has to emit both files, and
`ADR046-provider-001` acquires that obligation. The property bought is that the
digest in the catalog covers exactly the bytes a reviewer reads.

**Symlinked binaries are refused, which breaks `symlinkJoin` packaging.** A
Provider composed by joining several derivations must be repackaged so its
binaries are materialized in its own output. `buildRustPackage` already does
this; `symlinkJoin` and `buildEnv` do not.

**The unpinned remainder of the output is a residual, honestly.** Item 2 leaves
everything outside `bin/` and `share/d2b/provider/` unpinned by any digest. A
pinned binary can exec an unpinned sibling from its own closure. Item 7 bounds
that to "not framework-launchable", and the sandbox profile bounds it further,
but it is not closed by a digest and this ADR does not claim it is. Closing it
would require walking and hashing the whole output tree, which makes the package
digest and the closure digest redundant with each other and makes any
architecture-dependent build product a false failure.

**One signature is one revocation unit.** Because the manifest is the only
signed document, rotating a single binary requires re-signing the manifest and
producing a new artifact, which is exactly the intent of exact-digest selection.
It also means a compromised publisher key invalidates every artifact that key
signed, with no partial-trust story. That is the fail-closed side of the trade
and it is deliberate.

**Two provider dossiers become wrong on landing.** The transport-azure-relay
dossier's `transport-settings.schema.json` and the system-core dossier's
`provider-system-core-manifest.json` both contradict item 2 and must be amended
in the same change that lands the section 14.10 amendment, or the spec set will
carry three spellings of two files.

## Alternatives considered

**Multiple Nix outputs, with the manifest in a named `manifest` output.** This
reads naturally: `pkg.manifest` is self-documenting and the manifest is not
architecture-dependent. Rejected on the measurement above. `provider-catalog.nix`
coerces the package with `"${...}"`, which yields the first output; making the
catalog output-aware means either teaching it `outputName` handling at every
use, or requiring a specific first output, which is a fragile ordering
convention. The single-output rule is one eval assertion and it composes with
every existing consumer unchanged.

**Identity-parameterized file names, as the system-core dossier spells them.**
`provider-<name>-manifest.json` is friendlier when several manifests sit in one
directory, which never happens here. It forces the compiler to know the Provider
identity before it can open the file that declares the identity, and the only
identity available at that point is the operator-authored `artifactId`, which is
not the Provider name and is not attested by anything the publisher signed.
Deriving a path from an unverified string to find the document that verifies it
is the wrong direction.

**Manifest-declared paths for the schema and the binaries.** A `paths` object
inside the manifest is maximally flexible and lets a Provider lay itself out
however it likes. Rejected because it makes the required-outputs check
conditional on parsing the manifest first, which means a malformed manifest
produces "cannot check" rather than "required output absent", and because a
publisher-supplied relative path is a traversal surface the compiler would then
have to sanitize. Fixed constants have no traversal surface.

**Digest the parsed JSON rather than the file octets.** Canonicalizing before
hashing is tolerant of formatting and would let publishers pretty-print. It also
means two byte-different files share a digest, so the digest no longer attests
the artifact a reviewer reads, and a tampered file whose difference normalizes
away passes. Item 4 takes the strict side and requires the file to already be
canonical, which gets formatting tolerance's only real benefit - a single
well-defined form - without the ambiguity.

**Sign the whole output tree instead of the manifest.** A NAR-level signature
over the store path covers everything, including the unpinned remainder. It also
duplicates what the Nix store path already guarantees, requires the compiler to
reimplement NAR serialization to verify it, and produces a signature whose
preimage no reviewer can inspect. The manifest-anchored chain in item 6 gives the
same coverage for everything the framework actually reaches, over a document a
human can read.

**Leave the layout to each Provider dossier.** Fourteen dossiers already exist
and two of them already disagree. Per-dossier layout means the resource compiler
carries fourteen code paths, and a third-party Provider with no dossier has no
layout at all. The whole point of `d2b.artifacts` being a closed, offline,
exact-digest catalog is that one rule admits every artifact.

**Defer the whole thing until `ADR046-provider-002` settles the digest set.**
This was the status quo and it is what implementation-debt 19.7 recorded. It
holds three work items across two waves hostage to a catalog-field naming
question that is independent of where files live. Item 11 separates the two so
the layout can land now.
