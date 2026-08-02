# ADR 0050: Provider derivation artifact layout

- Status: Proposed
- Date: 2026-08-02
- Related: [ADR 0046](0046-d2b-3-provider-control-plane.md) (d2b 3.0 Provider
  control plane) and its decisions D012, D059, D075, D101, and D120 in
  [`docs/specs/ADR-046-decision-register.md`](../specs/ADR-046-decision-register.md);
  [ADR 0015](0015-daemon-only-clean-break.md) for the daemon-only control plane
  the resource plane sits inside; [ADR 0032](0032-d2b-v2-constellation-control-plane.md)
  and [ADR 0043](0043-realm-native-control-plane.md) for the rule that no host
  process holds realm credentials;
  [ADR 0034](0034-storage-lifecycle-restart-and-synchronization.md) for the
  anchored `openat2` fd-relative resolution discipline item 8 reuses;
  [ADR 0008](0008-supported-platforms-and-rejected-targets.md) for the kernel
  floor that makes `openat2` unconditionally available
- Scope: the on-disk shape of a Provider Nix derivation, the digest preimages
  that pin it, how the compiler and the launcher resolve a path inside it, the
  failure taxonomy they raise, and the Phase 1/Phase 2 checks that refuse a
  derivation which does not match. Consumed by `ADR046-zone-control-015` (the
  resource compiler), `ADR046-zone-control-016`, and `ADR046-zone-control-021`.

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

**The panel's counter-measurement, which falsified the first draft's rule.** The
first draft asserted `builtins.length package.all == 1`. Re-measuring the full
set of shapes `types.package` admits shows that predicate is both wrong and
partial:

| Value | `all` | `outputs` | `outputName` | `outputSpecified` |
| --- | --- | --- | --- | --- |
| `pkgs.hello` | len 1 | `["out"]` | `out` | absent |
| `pkgs.writeText "x" "y"` | len 1 | present | `out` | absent |
| `pkgs.openssl` (whole) | len 6 | 6 names | `bin` | **absent** |
| `pkgs.openssl.bin` | len **6** | 6 names | `bin` | **true** |
| `pkgs.openssl.dev` | len **6** | 6 names | `dev` | **true** |
| `lib.toDerivation "<store path>"` | **absent** | `["out"]` | `out` | absent |
| raw `derivation`, one output | **absent** | **absent** | `out` | absent |
| raw `derivation`, two outputs | len 2 | `["out","lib"]` | `out` | absent |
| raw `derivation`, `.lib` selected | len 2 | `["out","lib"]` | `lib` | absent |

Two failures follow. First, `all` has length 6 on `pkgs.openssl.dev`, which
names exactly one output, so the predicate **rejects a correctly pinned single
output**. Second, `all` is *absent* on two shapes `types.package` accepts -
`lib.types.package.check "${pkgs.hello}"` returns `true`, and the module system
coerces such a string through `lib.toDerivation`, whose result carries
`["name","out","outPath","outputName","outputs","type"]` and no `all` - so
`builtins.length package.all` **throws**, replacing an actionable module
assertion with a raw eval trace.

`outputs` is the attribute to test, defaulted, together with `outputSpecified`.
`outputs` is absent on exactly one admitted shape, the raw one-output primop,
where defaulting to `["out"]` is correct.

**The hazard is ambiguity of intent, not of the resulting path.** Worth stating
precisely, because it changes what the assertion is for: `pkgs.openssl` and
`pkgs.openssl.bin` coerce to the *same* store path. Nix is not choosing
nondeterministically. The hazard is that an operator writing
`package = pkgs.foo;` for a multi-output `foo` silently pins `foo`'s first
output without knowing it, and then reads a required-outputs failure as a bug in
a Provider that is packaged correctly.

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

### 1. A Provider artifact pins exactly one Nix output, named unambiguously

The catalog entry must resolve to one output, and the operator must have said
which one. `nixos-modules/provider-catalog.nix` asserts, for every
`d2b.artifacts.<id>` entry with `type = "provider"`:

```nix
let
  # `outputs` is absent on exactly one admitted shape, the raw one-output
  # primop, where "out" is the correct default. `all` is deliberately not
  # read: it is absent on store-path-coerced values and carries the whole
  # derivation's output count on an explicitly selected output.
  declaredOutputs = artifact.package.outputs or [ "out" ];
  outputSelected  = (artifact.package.outputSpecified or false) == true;
  shapeRecognised =
    builtins.isList declaredOutputs
    && builtins.all builtins.isString declaredOutputs
    && declaredOutputs != [ ];
in
  shapeRecognised && (outputSelected || builtins.length declaredOutputs == 1)
```

The predicate is **total** over every shape `types.package` admits, which the
table in Context enumerates: it reads only `outputs` and `outputSpecified`, both
defaulted, and it calls `builtins.length` only after `builtins.isList` has
succeeded. An unrecognised shape rejects rather than throws, so the operator
gets a module assertion instead of an eval trace.

It accepts a single-output derivation, and an explicitly selected output of a
multi-output derivation (`pkgs.foo.dev`). It rejects exactly one thing: a whole
multi-output derivation with no selection, which is the case that silently pins
the first output.

Three failure messages, each naming the artifact ID and, where known, the
observed output names:

| Condition | Message and remediation |
| --- | --- |
| Multi-output, unselected | `provider-artifact-output-ambiguous`: names the artifact ID and the declared output names, and instructs the operator to name one, for example `package = pkgs.<name>.out;` |
| Unrecognised shape | `provider-artifact-output-shape-unknown`: names the artifact ID and states that the value declares no usable output list |
| Empty output list | folded into the shape error above |

This is Phase 1, so it fails before any build starts. It earns its place even
though Phase 2 would also fail closed, because `nix-build-required-outputs-missing`
against an output the operator never meant to pin is a *misleading* diagnosis: it
points at the Provider's packaging rather than at the one-word omission in the
operator's own Nix.

Every path this ADR fixes is relative to the single store path that
`"${artifact.package}"` yields. That makes "the required outputs are present"
computable from the one value the catalog already records.

**Known limitation, recorded rather than papered over.** `outputSpecified` is a
nixpkgs `mkDerivation` convention. A multi-output derivation built with the raw
`builtins.derivation` primop and then selected (`rawMulti.lib`) carries
`outputName = "lib"` but no `outputSpecified`, and is therefore rejected. That is
fail-closed with an actionable message, and the remediation is to package the
Provider with `stdenv.mkDerivation` or one of its wrappers, which every Provider
crate does already.

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

**All three MUST be regular files.** A symlink is refused, and so is a directory,
a FIFO, a socket, or a device node. The rule is the same one item 3 applies to
`bin/` entries and for the same reason: a symlink can resolve outside the pinned
output, so its digest would cover bytes the package digest does not, and a
manifest reached through a link is not a manifest the pinned path contains. The
check is not a `stat` followed by an `open`; item 8 fixes how it is performed.

`bin/` exists if and only if the manifest's `package.executableDigests` is
non-empty. A Provider that ships no executable of its own - `system-core`, whose
handlers link into the `d2b-core-controller` binary - declares an empty
`executableDigests` and ships no `bin/`. A manifest whose components carry any
`binaryRef` MUST declare a non-empty `executableDigests`. A `bin/` directory that
exists but is empty is refused: it means the manifest and the derivation
disagree about whether this Provider ships executables, and silently treating it
as the empty set would let a build that dropped every binary pass.

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
bytes the package digest does not. A directory, FIFO, socket, or device node is
refused for the same reason.

Each `<name>` MUST match `^[a-z][a-z0-9-]*$` and be at most 64 bytes. The
grammar is checked against the **directory entry read from `<out>/bin`**, not
only against the manifest key, because the manifest key is a publisher claim and
the directory entry is what the launcher would resolve. Names outside the
grammar are refused rather than normalised: a name containing `/`, `.`, `..`, a
NUL, an ASCII control byte, whitespace, or a leading `-` is rejected, and so is
any name whose bytes are not valid UTF-8. This closes the argument-injection and
traversal shapes before `binaryRef` is ever concatenated with anything.

A mismatch fails the build naming the symmetric difference and the side each
name came from. An invalid name fails the build naming the offending entry
under its layout-relative path.

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

### 8. Every path in the layout is resolved fd-relative under an anchored dirfd

Neither the compiler nor the launcher resolves a layout path by building a string
and calling `stat` then `open`. Both anchor once and stay anchored.

**Anchor.** Open the pinned store path once with `O_PATH | O_DIRECTORY |
O_CLOEXEC`. Every subsequent resolution is `openat2(2)` relative to that dirfd
with `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`, and the
path argument is a fixed literal from item 2 or a single already-validated
`bin/<name>` component from item 3. `RESOLVE_BENEATH` makes escape from the
anchor a kernel-enforced `EXDEV` rather than a check the caller could forget.
`RESOLVE_NO_SYMLINKS` refuses intermediate *and* trailing symlinks, so the
regular-file rules in items 2 and 3 are enforced by the open itself rather than
by a preceding `lstat` whose result could be stale.
`RESOLVE_NO_MAGICLINKS` refuses `/proc/*/fd`-style jumps.

**Confirm after opening, not before.** Every opened fd is `fstat(2)`ed on the fd
and refused unless `S_ISREG` holds. The digest is then computed by reading *that
same fd*, never by reopening the path. There is no window between the check and
the use because there is no second resolution.

**Launch.** The launcher resolves `bin/<binaryRef>` through the same anchored
`openat2`, `fstat`s the fd for `S_ISREG`, and executes it with
`execveat(fd, "", argv, envp, AT_EMPTY_PATH)`. The program that runs is the
inode the digest was computed over, with no path re-traversal in between.

**This is not novel machinery.** It is the same discipline ADR 0034 already
binds for broker storage mutations, which AGENTS.md states as "anchored
`openat2`/fd-relative path walking" with "OFD locks with `O_CLOEXEC`, explicit fd
transfer only". This ADR reuses it rather than introducing a second convention.

**Nix store immutability is a reason to keep the discipline, not to skip it.**
A store path is immutable *after* it is registered, but the compiler and the
launcher do not run at that instant and cannot verify it. The Provider
derivation is an operator-supplied input that may be a locally built path, a
substituted path, or a path on a store whose daemon the operator controls, and
`/nix/store` is a normal directory that a privileged process can write. Anchored
resolution costs one extra fd and removes the entire class; declining it would
mean relying on an invariant this code has no way to check.

**Two portability facts, recorded because they are load-bearing.** `openat2`
requires Linux 5.6 and this repository's floor is 6.9 (ADR 0008), so it is
unconditionally available; there is no fallback path and a kernel without it
fails closed. And `execveat` with `AT_EMPTY_PATH` on an `O_PATH` fd needs
`/proc` mounted in the callee's mount namespace to run a `#!` script. Provider
component binaries are ELF images, which do not need it; a Provider shipping a
script entry point must therefore either be refused or run in a profile that
mounts `/proc`. This ADR takes the narrow side: `bin/` entries are ELF, and a
non-ELF entry is a Phase 2 build failure, so the launcher never meets the case.

### 9. One bounded, actionable failure taxonomy

Section 13.4 already binds the shape: stable kebab-case codes, messages bounded
at 512 bytes, UTF-8 validated, control-character sanitised, and carrying no
secret, credential, path, or process data. These codes join that table.

| Code | Raised when | Names |
| --- | --- | --- |
| `provider-artifact-output-ambiguous` | Multi-output package, no selection | artifact ID, declared output names |
| `provider-artifact-output-shape-unknown` | Package declares no usable output list | artifact ID |
| `provider-required-output-absent` | A file from item 2 is missing | artifact ID, layout-relative path |
| `provider-required-output-not-regular` | A layout path is a symlink, directory, or other non-regular type | artifact ID, layout-relative path, observed file type token |
| `provider-signature-publisher-unregistered` | `publisher` is neither `d2b-official` nor a declared trusted publisher | artifact ID, publisher, the option path `d2b.zones.<zone>.trustedPublishers.<publisher>` |
| `provider-signature-id-unresolvable` | `signatureId` names no key under a registered publisher | artifact ID, publisher, signatureId |
| `provider-signature-malformed` | The `.sig` file is not exactly 64 octets | artifact ID, expected length `64`, observed length |
| `provider-signature-verification-failed` | 64 well-formed octets that do not verify under the resolved key | artifact ID, publisher, signatureId |
| `provider-digest-mismatch` | Any pinned digest disagrees | artifact ID, which digest, expected value, actual value, and the two disagreeing sources |
| `provider-manifest-not-canonical` | File octets are not their own canonical bytes | artifact ID, layout-relative path, byte offset of first divergence, expected and observed lengths |
| `provider-executable-name-invalid` | A `bin/` entry violates the item 3 grammar | artifact ID, the rejected entry, the grammar |
| `provider-executable-set-empty` | `bin/` exists but has no entries | artifact ID |
| `provider-executable-set-mismatch` | Key set and directory set differ | artifact ID, the symmetric difference and the side of each name |
| `provider-binary-ref-unresolved` | A `binaryRef` is not a key | artifact ID, component ID, the ref |

**Four signature codes, not one, is the point.** "Signature chain invalid" is
unactionable: an unregistered publisher is fixed by editing
`d2b.zones.<zone>.trustedPublishers`, an unresolvable `signatureId` by
republishing or correcting the catalog entry, a malformed `.sig` by fixing the
build, and a verification failure by re-signing. Collapsing them makes the
operator guess which of four unrelated remediations applies.

**The bounded safe representation, and why each value is safe to name.** The
panel asked for actionable values; section 13.4 forbids leaking. These do not
conflict once each value is examined rather than the class assumed:

- `publisher` and `signatureId` are catalog fields already required to be
  bounded tokens (`^[a-z][a-z0-9-]*$`, non-empty). They are public artifact
  metadata, not credentials. Emitted whole.
- The `trustedPublishers` remediation is a **Nix option path**, not a filesystem
  path. It contains no host information and is the literal text the operator
  must type.
- Digests are fixed-width 71-byte `sha256:<64 hex>` tokens, so "bounded" holds by
  construction. Both expected and actual are emitted **in full**. Truncating them
  would be security theatre: a digest of a world-readable store file discloses
  nothing, and a truncated digest is materially harder to diagnose from.
- Layout-relative paths (`share/d2b/provider/provider-manifest.json`) are fixed
  literals of item 2. They are not host paths and reveal nothing about the
  machine. **`<out>` and any absolute path are never emitted**, which is the
  clause of section 13.4 that actually binds.
- The canonical-JSON failure emits a **byte offset and two lengths**, never file
  content. Bounded by construction, and it is what makes the failure fixable.
  The remediation text is fixed: "re-emit with the toolkit canonical serializer;
  the usual cause is a trailing newline."
- Key material, manifest contents, config values, and store paths are never
  emitted by any code above.

Errors surfaced at Phase 2 and errors surfaced later into status, audit, or OTEL
use **one** taxonomy, deliberately. A build message is an operator terminal and a
status message is a redaction boundary; maintaining two vocabularies is how a
value that was safe in the first ends up in the second. Writing to the stricter
contract once costs nothing.

### 10. Nothing in the required layout may be sensitive

The derivation is world-readable in `/nix/store`. The manifest and the config
schema carry no credential, no token, no key, no host path, and no PID. Secrets
remain `Credential/<name>` refs resolved at runtime, exactly as section 4.3.2
already requires of `spec.config`. The trust root material the compiler reads is
public verification keys. No private key and no realm credential enters
`d2b.artifacts`, the host bundle, or any host-side activation artifact.

### 11. The checks and their conformance scenarios

The missing scenarios are added to section 15.8. Each is named so
`ADR046-zone-control-015`'s validation field can cite it.

| Scenario | Phase | Assertion |
| --- | --- | --- |
| `nix-eval-provider-output-ambiguous` | 1 | A `type = "provider"` artifact whose package is a whole multi-output derivation with no explicit output selection fails eval naming the artifact ID and the declared output names; the same package with an output selected (`pkgs.foo.out`) evaluates |
| `nix-eval-provider-output-shape-unknown` | 1 | A `type = "provider"` artifact whose package declares no usable output list, including a store-path-valued `types.package`, fails eval with a module assertion rather than an eval trace |
| `nix-build-required-outputs-missing` | 2 | A Provider derivation missing any of `share/d2b/provider/provider-manifest.json`, `provider-manifest.json.sig`, or `config-schema.json` fails build naming the absent layout-relative path |
| `nix-build-required-output-not-regular` | 2 | Each of the three `share/d2b/provider/` paths, replaced in turn by a symlink (including one resolving inside the same output) and by a directory, fails build naming the path and the observed file type |
| `nix-build-manifest-signature-invalid` | 2 | Four distinct cases fail with four distinct codes: unregistered publisher, unresolvable `signatureId`, a `.sig` that is not exactly 64 octets, and 64 well-formed octets that do not verify |
| `nix-build-manifest-not-canonical` | 2 | A manifest or config schema whose octets are not their own `d2b-cjson/v1` canonical bytes fails build naming the file and the first divergent byte offset; a trailing newline alone is sufficient to fail |
| `nix-build-executable-set-mismatch` | 2 | `executableDigests` keys unequal to the `bin/` entry set fails build naming the symmetric difference |
| `nix-build-executable-set-empty` | 2 | A derivation with a `bin/` directory containing no entries fails build, and is distinguished from a Provider that legitimately declares an empty `executableDigests` and ships no `bin/` at all, which succeeds |
| `nix-build-executable-name-invalid` | 2 | A `bin/` entry whose name violates `^[a-z][a-z0-9-]*$`, exceeds 64 bytes, contains `/`, `.`, `..`, NUL, an ASCII control byte, or whitespace, begins with `-`, or is not valid UTF-8, fails build naming the entry; the name is checked as read from the directory, not only as declared in the manifest |
| `nix-build-executable-not-regular-file` | 2 | A `bin/` entry that is a symlink, directory, FIFO, socket, or device node fails build naming the entry |
| `nix-build-binary-ref-unresolved` | 2 | A component `binaryRef` absent from `executableDigests` fails build naming the component and the ref |
| `nix-build-executable-digest-mismatch` | 2 | A `bin/` file whose SHA-256 differs from its `executableDigests` value fails build naming the binary and both digests in full |
| `nix-build-catalog-manifest-disagreement` | 2 | Operator-authored catalog digests, manifest-declared digests, and compiler-recomputed digests disagreeing on any pinned value fails build naming the two disagreeing sources and both digest values |
| `nix-build-provider-error-redaction` | 2 | No failure message from any code in item 9 contains an absolute path, a `/nix/store` prefix, key material, manifest content, or a config value, and every message is within the section 13.4 512-byte bound |

### 12. The scenario names two work items cite are corrected

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

### 13. What this ADR does not decide

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

### 14. A prerequisite this ADR depends on and does not itself supply

Item 3's `binaryRef` check cannot be written against the contract as it stands.
The shipped `ComponentDescriptor` in `packages/d2b-contracts/src/v3/provider.rs`
carries `component_id`, `component_type`, `exported_resource_types`,
`exported_methods`, `allowed_domains`, `cardinality`, `config_digest`,
`dependencies`, `declares_state_volume`, and `state_namespaces`. It has **no
`binary_ref` field at all**, while section 4.3.3 defines `binaryRef` three times
as a normative descriptor field.

That drift predates this ADR and this ADR does not create it, but item 3 depends
on it, so recording it silently as "someone else's problem" would leave T174 with
the same shape of hole it was blocked on. It is therefore carried as an explicit
W5 implementation obligation: `ComponentDescriptor` gains `binary_ref`, bounded
by the item 3 grammar, before `nix-build-binary-ref-unresolved` is implementable.
Until it lands, that one scenario is blocked while the other thirteen are not,
and the amendment records the split rather than letting the whole item slip.

## Consequences

**A Provider package becomes a checkable object rather than an assertion.**
`ADR046-zone-control-015` can now write the Phase 2 required-outputs check
against fixed paths, and every one of the fourteen scenarios in item 11 is a
filesystem or digest comparison a machine evaluates. The blocking status recorded
in implementation-debt 19.7 lifts for `015`, and with it for `016` and `021` -
with one exception, `nix-build-binary-ref-unresolved`, which item 14 records as
blocked on a contract field that does not exist yet.

**Multi-output Provider packaging is permitted but must be said out loud.** This
is where panel round 1 moved the decision. A Provider may ship a multi-output
derivation; the operator must select the output that carries the Provider
(`package = pkgs.foo.out;`). The cost is a small authoring burden on every
multi-output Provider and one more thing for a `d2b.artifacts` example to show.
The benefit is that the framework no longer refuses a correctly pinned
`pkgs.foo.dev`, which the first draft's `all`-based predicate did.

**Byte-canonical JSON files are a real authoring burden.** A publisher cannot
hand-write `provider-manifest.json` and have it pass; it must be emitted by a
canonicalizing serializer, with no trailing newline, which will surprise anyone
who opens it in an editor that adds one. The toolkit has to emit both files, and
`ADR046-provider-001` acquires that obligation. The property bought is that the
digest in the catalog covers exactly the bytes a reviewer reads. Item 9 makes the
failure survivable by naming the first divergent byte offset and the remediation
rather than saying "not canonical".

**Symlinks are refused throughout the layout, which breaks `symlinkJoin`
packaging.** Items 2 and 3 both require regular files, and item 8 enforces it
with `RESOLVE_NO_SYMLINKS` rather than a preceding `lstat`. A Provider composed
by joining several derivations must be repackaged so its binaries and its
manifest are materialized in its own output. `buildRustPackage` already does
this; `symlinkJoin` and `buildEnv` do not.

**Anchored resolution costs one fd and one syscall convention.** Item 8 requires
`openat2` with `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`,
`fstat` on the fd, and `execveat` from the fd. That is a narrower interface than
`std::fs` offers, so the compiler and the launcher both need a small anchored-fd
helper rather than path strings. It also forecloses escape hatches a future
maintainer might reach for, such as "just canonicalize the path first".

**The unpinned remainder of the output is a residual, honestly.** Item 2 leaves
everything outside `bin/` and `share/d2b/provider/` unpinned by any digest. A
pinned binary can exec an unpinned sibling from its own closure. Item 7 bounds
that to "not framework-launchable", item 8 makes the launcher's own resolution
non-bypassable, and the sandbox profile bounds it further, but it is not closed
by a digest and this ADR does not claim it is. Closing it would require walking
and hashing the whole output tree, which makes the package digest and the closure
digest redundant with each other and makes any architecture-dependent build
product a false failure.

**One signature is one revocation unit.** Because the manifest is the only
signed document, rotating a single binary requires re-signing the manifest and
producing a new artifact, which is exactly the intent of exact-digest selection.
It also means a compromised publisher key invalidates every artifact that key
signed, with no partial-trust story. That is the fail-closed side of the trade
and it is deliberate.

**Fourteen error codes is a real surface.** Item 9 adds fourteen rows to section
13.4's table, where a single `provider-package-invalid` would have added one.
Each one has to be tested for its redaction behaviour, which is what
`nix-build-provider-error-redaction` costs. The judgement is that an operator who
reads `provider-signature-publisher-unregistered` with the option path in the
message fixes it in one step, and an operator who reads "signature chain invalid"
opens an issue.

**Two provider dossiers become wrong on landing, and one becomes
self-contradictory before that.** The transport-azure-relay dossier's
`transport-settings.schema.json` and the system-core dossier's
`provider-system-core-manifest.json` both contradict item 2. The system-core
dossier additionally declares `binaryRef: d2b-core-controller` on both of its
component descriptors while shipping no `bin/`, which item 3 would fail. All
three must be amended in the same change that lands the section 14.10 amendment.

## Alternatives considered

**Multiple Nix outputs, with the manifest in a named `manifest` output.** This
reads naturally: `pkg.manifest` is self-documenting and the manifest is not
architecture-dependent. Rejected, but narrowly: the layout must live in **one**
output because `provider-catalog.nix` records `storePath = "${artifact.package}"`
and that is the only path the catalog pins, digests, and stages. Splitting the
manifest into a second output would put it outside the digested path. What the
decision permits, after panel round 1, is a multi-output derivation whose
Provider-carrying output the operator names explicitly; what it refuses is
spreading the required layout across several of them.

**Requiring exactly one Nix output, tested with `builtins.length package.all`.**
This was the first draft's rule and it was wrong twice, which the Context
measurement table records: `all` is length 6 on `pkgs.openssl.dev`, so a
correctly pinned single output is refused, and `all` is absent on a
store-path-valued `types.package`, so the assertion throws instead of asserting.
The replacement in item 1 reads `outputs` and `outputSpecified`, both defaulted,
and is total over every measured shape.

**Testing `outputName` against the first element of `outputs`.** An appealing
way to detect "the operator did not choose": if `outputName == head outputs` the
selection might be the implicit default. Rejected because it is not a test of
intent at all - `pkgs.openssl.bin` satisfies it while being an explicit choice -
so it would reject exactly the case item 1 exists to permit. `outputSpecified` is
the only attribute that records whether a human named the output.

**Identity-parameterized file names, as the system-core dossier spells them.**
`provider-<name>-manifest.json` is friendlier when several manifests sit in one
directory, which never happens here. It forces the compiler to know the Provider
identity before it can open the file that declares the identity, and the only
identity available at that point is the operator-authored `artifactId`, which is
not the Provider name and is not attested by anything the publisher signed.
Deriving a path from an unverified string to find the document that verifies it
is the wrong direction.

**Trusting Nix store immutability and resolving layout paths by string.** The
store is immutable after registration, so a plain `File::open` on a joined path
looks safe and is far less code than item 8. Rejected because neither the
compiler nor the launcher can verify that precondition at the moment it matters:
the artifact is an operator-supplied input that may be locally built or
substituted, `/nix/store` is a normal directory writable by a privileged process,
and the launcher runs long after the compiler checked. Anchored `openat2` costs
one fd and removes the class rather than arguing about it, and ADR 0034 already
binds the same discipline for broker storage mutations, so it is a convention
this repository keeps rather than one this ADR invents.

**One `provider-package-invalid` error code instead of fourteen.** Smaller table,
smaller test surface, and it is what most of the existing section 13.4 rows do
for coarse failures. Rejected because the four signature cases alone have four
unrelated remediations, and a code that cannot tell an operator whether to edit
`trustedPublishers`, re-sign, or fix the build is a code that generates an issue
instead of a fix.

**Truncating digests in error messages for safety.** Suggested by the instinct
that hashes are sensitive. Rejected after examining the value rather than the
class: these digests cover world-readable `/nix/store` files, so they disclose
nothing, they are fixed-width 71-byte tokens so the bounding concern is already
satisfied by construction, and a truncated digest makes the one thing the
operator needs to compare materially harder to compare. The redaction that does
bind is the absolute-path and key-material ban, which item 9 states explicitly.

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
question that is independent of where files live. Item 13 separates the two so
the layout can land now.
