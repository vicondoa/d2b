# Amendment request: Provider derivation artifact layout

| Field | Value |
| --- | --- |
| Scope | The required derivation outputs of a Provider artifact: output name, file paths, executable set, digest preimages, signature anchoring, and the conformance scenarios that check them |
| Raised under | The W5 audit, recorded in `implementation-debt.md` sections 12.1, 12.2, 12.3, 14.8, and 19.7 |
| Deciding record | [ADR 0050](../../docs/adr/0050-provider-derivation-artifact-layout.md), currently **Proposed** |
| Affected member specs | `ADR-046-resources-zone-control` (sections 4.3.1, 4.9 new, 13.4, 14.10, 15.8, 17); `ADR-046-provider-model-and-packaging` (Package catalog, Crate/package boundary); `ADR-046-nix-configuration` (Validation); `ADR-046-security-and-threat-model`; `ADR-046-decision-register` (D101 domain tags); provider dossiers `system-core` (naming **and** a `binaryRef` self-contradiction) and `transport-azure-relay` |
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

#### 4.9.1 One unambiguously named Nix output

A `type = "provider"` artifact's package MUST resolve to exactly one Nix output,
and where the derivation has more than one, that output MUST carry evidence that
it was chosen. The Phase 1 assertion, evaluated in
`nixos-modules/provider-catalog.nix`, is:

```nix
let
  package = artifact.package;
  declaredOutputs = package.outputs or [ "out" ];
  shapeRecognised =
    builtins.isList declaredOutputs
    && declaredOutputs != [ ]
    && builtins.all builtins.isString declaredOutputs;
in
  if !shapeRecognised then false
  else if builtins.length declaredOutputs == 1 then true
  else if (package.outputSpecified or false) == true then true
  else (package.outputName or null) != builtins.head declaredOutputs
```

The predicate MUST NOT read `package.all`. Measured across every shape
`lib.types.package` admits:

| Value | `all` | `outputs` | `outputName` | `outputSpecified` |
| --- | --- | --- | --- | --- |
| `pkgs.hello` | len 1 | `["out"]` | `out` | absent |
| `pkgs.openssl` (whole) | len 6 | 6 names | `bin` | absent |
| `pkgs.openssl.bin` | len **6** | 6 names | `bin` | **true** |
| `pkgs.openssl.dev` | len **6** | 6 names | `dev` | **true** |
| `pkgs.openssl.out` | len 6 | 6 names | `out` | **true** |
| `lib.toDerivation "<store path>"` | **absent** | `["out"]` | `out` | absent |
| raw `derivation`, one output | **absent** | **absent** | `out` | absent |
| raw `derivation`, two outputs | len 2 | `["out","lib"]` | `out` | absent |
| raw `derivation`, `.lib` selected | len 2 | `["out","lib"]` | **`lib`** | absent |
| raw `derivation`, `.out` selected | len 2 | `["out","lib"]` | `out` | absent |

`all` therefore rejects `pkgs.openssl.dev`, which names exactly one output, and
throws on a store-path-valued `types.package`, which `lib.types.package.check`
accepts and the module system coerces through `lib.toDerivation`. The predicate
above reads only defaulted attributes, and `if` ordering places the
`shapeRecognised` guard before every partial operation, so `builtins.head` and
`builtins.length` are reached only on a non-empty list of strings. It is total,
and an unrecognised shape rejects with a module assertion rather than an eval
trace.

**A store-path-valued package is accepted.** `lib.toDerivation` yields
`outputs = ["out"]`, one determinate output, and the layout check, the digests,
and the signature chain all run against that path unchanged. There is no security
reason to refuse it, so the predicate's existing behaviour governs and the prose
states it.

**`outputName` is the second witness of selection.** For a raw-primop
derivation, `.lib` reports `outputName = "lib"` while the whole derivation
reports `outputName = "out"`, the head of `outputs`. `outputName != head outputs`
therefore establishes explicit selection for exactly the shapes
`outputSpecified` does not reach.

The rule exists because `provider-catalog.nix` records
`storePath = "${artifact.package}"`, and for a whole multi-output derivation that
string is the **first** output. The resulting path is determinate, so the hazard
is not nondeterminism: it is that the operator did not know which output was
pinned, and would read a later required-outputs failure as a Provider packaging
bug rather than as a missing output selector in their own Nix.

Failures, both fail-closed:

| Condition | Error code | Message names |
| --- | --- | --- |
| Multi-output package pinning its first output with no evidence of selection | `provider-artifact-output-ambiguous` | artifact ID, the declared output names, and the remedy for the case at hand: on a `stdenv.mkDerivation` derivation select any output (`package = pkgs.<name>.out;`), which sets `outputSpecified`; on a raw `builtins.derivation` selecting a **non-first** output already satisfies the check through the `outputName` witness; only wanting the raw primop's **first** output requires repackaging with `stdenv.mkDerivation` |
| `outputs` present but not a non-empty list of strings | `provider-artifact-output-shape-unknown` | artifact ID; the remedy is to supply a derivation or a store path rather than a hand-built attrset |

The message MUST NOT tell an operator to select an output when selection cannot
produce the evidence the predicate reads, and it MUST NOT tell an operator to
repackage when selection would suffice. Selecting a non-first output on a raw
primop derivation **does** satisfy the check, through the `outputName` witness;
only the raw primop's first output is unreachable by selection.

Residual case, stated exactly: a raw-primop multi-output derivation whose
**first** output is explicitly selected is indistinguishable from the whole
derivation - same `outputName`, same head of `outputs`, no `outputSpecified` -
and both coerce to the same store path, so both are refused with the same answer.
The remedy above is truthful for it.

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
| `<out>/share/d2b/provider/provider-manifest.json` | The signed Provider manifest of section 4.3, serialized as `d2b-cjson/v1` with no trailing newline. MUST be a regular file | `provider-required-output-absent`; `provider-required-output-not-regular` |
| `<out>/share/d2b/provider/provider-manifest.json.sig` | Exactly 64 octets: a detached Ed25519 signature over the manifest file's raw octets. No framing, no base64, no trailing newline. MUST be a regular file | `provider-required-output-absent`; `provider-required-output-not-regular`; `provider-signature-malformed` if the length is wrong |
| `<out>/share/d2b/provider/config-schema.json` | The root JSON Schema that `spec.config` is validated against (section 4.3.2), serialized as `d2b-cjson/v1` with no trailing newline. MUST be a regular file | `provider-required-output-absent`; `provider-required-output-not-regular` |
| `<out>/bin/<name>` | One regular file per built component executable. Present if and only if `package.executableDigests` is non-empty | `provider-executable-set-mismatch`; `provider-executable-not-regular`; `provider-executable-name-invalid` |

**All three files under `share/d2b/provider/` MUST be regular files.** A symlink
is refused, and so is a directory, FIFO, socket, or device node. A symlink can
resolve outside the pinned output, so its digest would cover bytes the package
digest does not, and a manifest reached through a link is not a manifest the
pinned path contains. Section 4.9.7 fixes how the file type is established; it is
never an `lstat` followed by a separate `open`.

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

**A `<out>/bin` directory that exists but is empty is refused**
(`provider-executable-set-empty`). It means the manifest and the derivation
disagree about whether this Provider ships executables, and treating it as the
empty set would let a build that dropped every binary pass silently. Shipping no
`bin/` at all, with an empty `package.executableDigests`, is the supported way to
express "this Provider has no executable of its own".

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
package digest. A directory, FIFO, socket, or device node is refused for the same
reason. Section 4.9.7 fixes how the file type is established.

Each `<name>` MUST match `^[a-z][a-z0-9-]*$` and be at most 64 bytes. The
grammar is checked against the **directory entry read from `<out>/bin`**, not
only against the manifest key, because the manifest key is a publisher claim
while the directory entry is what the launcher would resolve. A name containing
`/`, `.`, `..`, NUL, an ASCII control byte, or whitespace, a name beginning with
`-`, and a name whose bytes are not valid UTF-8 are all rejected
(`provider-executable-name-invalid`) rather than normalised. This closes the
traversal and argument-injection shapes before `binaryRef` is concatenated with
anything.

`<out>/bin` MUST NOT exist with zero entries; see 4.9.2.

**Every `<out>/bin/<name>` MUST be an ELF executable.** The check is a bounded
read of the first 18 octets from the already-open descriptor: `e_ident` begins
`\x7fELF`, `EI_CLASS` is `ELFCLASS64`, `EI_DATA` matches host byte order,
`EI_VERSION` is 1, and `e_type` is `ET_EXEC` or `ET_DYN`. No parser runs, nothing
is mapped, nothing is executed. `ET_REL`, `ET_CORE`, a `#!` script, an empty
file, and a file shorter than the header are all refused with
`provider-executable-not-elf`, naming the entry and the first four octets as hex
and pointing at the shim builder of 4.9.3a.

The requirement is not stylistic. `execveat` with `AT_EMPTY_PATH` on an `O_PATH`
descriptor needs `/proc` mounted in the callee's mount namespace to start a `#!`
interpreter (see 4.9.7), and d2b sandbox profiles do not promise one. Refusing
non-ELF entries at build time means the same artifact behaves identically on
every host, instead of succeeding where `/proc` happens to be mounted.

##### 4.9.3a `d2b.lib.buildProviderElfShim` (normative, framework-owned)

The framework MUST ship a helper that produces a conforming ELF entry point for
a Provider whose component is an interpreted program, so that the ELF rule of
4.9.3 has a supported route rather than only a prohibition:

```nix
d2b.lib.buildProviderElfShim {
  inherit pkgs;
  name            = "d2b-provider-foo-controller";  # the bin/ entry name
  interpreterPkg  = pkgs.python3;                   # the package output
  interpreterPath = "bin/python3";                  # relative, inside that output
  program         = ./controller.py;
  extraArgs       = [ ];
}
```

The interpreter is supplied as **a package output plus a path relative to it**,
never as one interpolated absolute string, because the output is the boundary the
symlink walk of property 2 may not leave.

The result is a derivation whose `$out/bin/<name>` is a compiled `ET_DYN` image,
not a `#!` line and not a `makeWrapper` shell script.

Seven properties are normative:

1. the interpreter and the program are resolved at build time and baked in as
   string literals, split into a directory and a final component; the shim takes
   no argument and reads no environment variable that selects what to execute;
2. at build time the helper resolves the interpreter through a **bounded symlink
   chain confined to the same store output**, requiring the chain to terminate at
   a regular file that is an `ET_EXEC` or `ET_DYN` ELF64 image **and carries an execute bit** (`st_mode & 0o111 != 0`), since the shim will `execveat` it and a valid ELF without the bit yields `EACCES` at launch.
   Every link must be relative; an absolute target, any `..` component, a target
   leaving the output, and a chain longer than 8 links are refused. The
   **resolved** same-output relative path is what is baked.

   This is required by the ecosystem rather than chosen for convenience:
   `<python3-output>/bin/python3` is a relative same-output symlink to
   `python3.13`, whose first octets are `7f 45 4c 46`, so a flat no-symlink rule
   would reject the most common interpreter. Confining the walk to one immutable
   output preserves the closure identity the shim depends on: nothing outside the
   content-addressed path is consulted, and there is no `realpath` call and no
   cross-store canonicalization.

   **Wrapper-script interpreters are unsupported.** The same directory holds
   entries such as `idle3.13` whose first octets are `23 21 2f 6e`, a `#!` line.
   A chain ending at one fails the build, naming the entry and stating that a
   shebang wrapper cannot serve as a shim interpreter; the remedy is to name the
   real interpreter binary. This is a real limitation, stated rather than
   elided: a wrapper would return the framework to the `execveat`-on-script
   behaviour 4.9.3 exists to avoid;
3. the program is verified to be a regular file by the same bounded walk, and is
   not required to be ELF since the interpreter reads it; the resolved
   interpreter and the program are **members of the shim derivation's runtime
   closure**, so Nix guarantees their presence and the closure digest of 4.9.4
   covers them;
4. at runtime the shim resolves the baked interpreter path with the **same
   anchored fd discipline of 4.9.7**: the baked directory opened
   `O_PATH | O_DIRECTORY | O_CLOEXEC`, the final component opened via `openat2`
   with `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS` and
   `O_PATH | O_CLOEXEC`, `fstat` for `S_ISREG`, then
   `execveat(fd, "", argv, envp, AT_EMPTY_PATH)`. There is **no `execve` on a
   concatenated string**, no `PATH` search, no `execvp`, and no shell. Because
   the baked path is the already-resolved target, no chain is walked at runtime
   and `RESOLVE_NO_SYMLINKS` is the correct runtime posture;
5. caller `argv[1..]` is forwarded after the fixed arguments and is never
   interpreted as a program;
6. `name` is validated at eval against the 4.9.3 grammar, so the helper cannot
   emit an entry the resource compiler would reject for its name;
7. the derivation self-checks its own output with `readelf -h`, requiring
   `ELFCLASS64` and an `e_type` of `ET_EXEC` or `ET_DYN`, and fails rather than
   emitting something Phase 2 would refuse.

Property 7 reuses the `postInstall` ELF-assertion pattern `flake.nix` already
applies to the static guest helpers rather than introducing a second one.

**The framework still launches exactly one thing: the verified shim.** 4.9.3 and
4.9.7 are unchanged. The shim's subsequent exec of an interpreter is a second
exec by an already-launched, already-verified program inside its own sandbox
profile, which is the position any Provider binary occupies when it spawns a
child. The helper does not widen what the framework will launch.

**The exception is bounded to the framework-generated shim.** Only a shim emitted
by `d2b.lib.buildProviderElfShim` carries the build-time verification and closure
guarantees of properties 2 and 3. A hand-written Provider binary that execs a
sibling receives no exception and no special standing; it is bounded by its
sandbox profile alone. Nothing here licenses a general rule that Providers may
exec arbitrary paths.

**Why the baked store path is not the string-path redirection it resembles.** A
store path's hash derives from its derivation inputs, so changing the
interpreter's content yields a *different* path; there is no "same path,
different content" state Nix will produce. The path is in the shim's closure, so
it cannot become dangling and be recreated as something else. Rebinding it
requires writing to `/nix/store`, which NixOS mounts read-only and which
otherwise requires root, and an attacker who can write the store can replace the
shim, the Provider, and `d2bd` alike, so the shim's exec target is not the
boundary that failed. Property 4 does not rest on those facts alone, which is the
point: it resolves the interpreter under `RESOLVE_NO_SYMLINKS`, so a symlink
appearing at that path is refused at runtime rather than trusted.

**What the helper does not cover, exactly.** The interpreter is verified and
executed under fd discipline; the *program* the interpreter subsequently reads is
not, because the interpreter opens it by path and no framework code sits between.
The program is covered by build-time regular-file verification, canonical-path
baking, and closure membership, and by nothing stronger. Neither input is named
in `package.executableDigests`, because neither is a `bin/` entry. That is the
residual 4.9.2 already records, narrowed rather than widened by this helper.

A Process created for a Provider component resolves its program as
`<out>/bin/<binaryRef>` and by no other means: no `PATH` lookup, no
manifest-supplied absolute path, no path relative to a working directory.

**`BinaryRef` is a validated newtype and launchability is modelled.** The
`binaryRef` term of (3) is not implementable against the contract as committed.
`ComponentDescriptor` in `packages/d2b-contracts/src/v3/provider.rs` declares
`component_id`, `component_type`, `exported_resource_types`, `exported_methods`,
`allowed_domains`, `cardinality`, `config_digest`, `dependencies`,
`declares_state_volume`, and `state_namespaces`, and carries no `binary_ref`
field, while section 4.3.3 defines `binaryRef` three times as normative.

The field this amendment requires is not a `String` and not an
`Option<BinaryRef>`:

```rust
/// A `^[a-z][a-z0-9-]*$` component binary name, at most 64 bytes.
/// Parsing is the only constructor, so the grammar above is a type invariant.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct BinaryRef(BoundedToken);

/// Deserialization MUST route through the validating parser, mirroring
/// `ArtifactId` in the same module. A derived `transparent` impl would accept
/// any string from a signed manifest, leaving the grammar enforced only for
/// values constructed in Rust.
impl<'de> Deserialize<'de> for BinaryRef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

pub enum ComponentExecution {
    /// A Process is created and `<out>/bin/<binary_ref>` is launched.
    Launchable { binary_ref: BinaryRef },
    /// Handlers run in-process inside a binary from another derivation;
    /// no Process is created (§11.3 step 5).
    InProcessBootstrap,
}
```

**Wire representation: the flat `binaryRef` field is unchanged and the schema is
not versioned.** `ComponentExecution` is an in-memory invariant, not a new JSON
shape. No serde enum representation is used - not `tag`, not `untagged`, not
adjacent - because each would change the manifest bytes section 4.3.3 already
specifies, forcing a `schemaVersion` bump and a `manifestDigest` change on every
existing artifact. The descriptor is deserialized through a private wire struct
carrying `#[serde(default)] binary_ref: Option<BinaryRef>`, mapped once at the
parse boundary: present becomes `Launchable`, absent becomes
`InProcessBootstrap`. Serialization is the inverse, with `InProcessBootstrap`
omitting the field via `skip_serializing_if`, so a descriptor round-trips to the
bytes it was parsed from and `manifestDigest` is unaffected. Existing manifests
parse unchanged. **No `schemaVersion` or `apiVersion` bump is required.**

The ambiguity is not reintroduced but relocated to the only place it can be
resolved. On the wire a field is present or absent and nothing further can be
said; whether an absence is legitimate is a fact about the Provider, not the
JSON. The parse boundary names the variant and admission decides whether it was
allowed, so after admission there is exactly one in-memory representation of each
state and no `Option` for a caller to misread.

`Option<BinaryRef>` as the descriptor field is rejected because `None` would mean
both "not launchable" and "launchable but the manifest omitted the field", and
the second is the hole this check exists to close. `ComponentDescriptor` holds
one `ComponentExecution`, so launchability and the presence of a binary name are
the same fact.

Two admission rules bound the bootstrap arm, both raising
`provider-component-execution-invalid`:

1. `InProcessBootstrap` is admissible only for a Provider on the closed
   bootstrap-exception list (`system-core`, `system-minijail`); any other
   Provider omitting `binaryRef` is refused at manifest admission;
2. a Provider whose components are all `InProcessBootstrap` MUST declare an empty
   `package.executableDigests` and ship no `bin/`, and conversely.

`BinaryRef` is distinct from `ArtifactId` and `ComponentId` even though the three
grammars coincide today, because they are three namespaces and syntactic
coincidence is not a licence to substitute.

The drift predates this amendment. Two obligations gate four scenarios, stated as
explicit lists rather than totals because a count is what goes stale first:

| Obligation | Gates exactly |
| --- | --- |
| `ComponentDescriptor` gains `ComponentExecution` with `BinaryRef`, the flat-wire parse boundary, and the validating `Deserialize` | `nix-build-binary-ref-unresolved`, `nix-build-component-execution-invalid`, `nix-build-executable-declaration-inconsistent`, `nix-build-manifest-binary-ref-wire-compatible` |
| `d2b.lib.buildProviderElfShim` exists, per 4.9.3a | `nix-build-elf-shim-satisfies-the-layout` |

The seventeen scenarios gated by neither, named so the list can be checked:
`nix-eval-provider-output-ambiguous`,
`nix-eval-provider-output-shape-accepted`,
`nix-eval-provider-output-shape-unknown`, `nix-build-required-outputs-missing`,
`nix-build-required-output-not-regular`, `nix-build-executable-not-elf`,
`nix-build-executable-not-executable`,
`nix-build-manifest-signature-invalid`, `nix-build-manifest-not-canonical`,
`nix-build-executable-set-mismatch`, `nix-build-executable-set-empty`,
`nix-build-executable-name-invalid`, `nix-build-executable-not-regular-file`,
`nix-build-executable-digest-mismatch`,
`nix-build-catalog-manifest-disagreement`, `nix-build-provider-error-redaction`,
and `nix-runtime-launcher-anchored-resolution`. Twenty-two in total.

#### 4.9.4 Digest preimages

Every value renders `sha256:<64 lowercase hex>`.

| Value | Carried by | Preimage |
| --- | --- | --- |
| `package.executableDigests[<name>]` | signed manifest | SHA-256 over the raw octets of `<out>/bin/<name>` |
| `executableDigest` | artifact catalog | `canonical_digest("d2b:v3:provider-executable-set", C)`, where `C` is the `d2b-cjson/v1` serialization of the **whole** `package.executableDigests` object: every binary name as a key bound to that binary's own `sha256:<hex>` value |
| `manifestDigest` | artifact catalog | SHA-256 over the raw octets of `<out>/share/d2b/provider/provider-manifest.json` |
| `configSchemaDigest` | artifact catalog and signed manifest | SHA-256 over the raw octets of `<out>/share/d2b/provider/config-schema.json` |
| `digest` | artifact catalog | SHA-256 of the NAR serialization of `<out>`, as `nix hash path --type sha256 --base16` renders it |

Executable digests carry no domain tag: an ELF image is not canonical JSON and
there is nothing to canonicalize. The executable set digest is domain-separated
under D101 using the existing `canonical_digest` helper,
`SHA-256(domain_tag || 0x00 || canonical_bytes)`.

**Import the right helper.** Two functions in the workspace are named
`canonical_digest`. `packages/d2b-contracts/src/v3/resource_schema.rs:518` is the
D101 contract digest and hashes `domain_tag || 0x00 || canonical_bytes`;
`packages/xtask/src/delivery/model.rs:591` belongs to the delivery tooling,
hashes `domain || payload_len_u64_be || bytes`, and takes plain
`serde_json::to_vec` output rather than `d2b-cjson/v1` canonical bytes. This
amendment means the first. The second is not a D101 digest and must not be used
for any artifact in this layout.

The set digest binds the **map**, not a summary of it. The preimage is the
serialization of the object

```json
{"d2b-provider-foo-controller":"sha256:<64 hex>","d2b-provider-foo-service":"sha256:<64 hex>"}
```

covering every binary name, every per-binary digest, and the pairing between
them. It is not a digest of the name list, of the concatenated digest values, or
of a count or tuple. Key order is not a degree of freedom: `d2b-cjson/v1` is
RFC 8785 JCS narrowed, so object keys are sorted by code unit during
serialization and one map has one digest regardless of emission order. Renaming
a binary, changing one binary's bytes, adding one, or removing one all change
this value; reordering the authored map does not.

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
`publisher` and `signatureId`.

Four distinct conditions produce four distinct error codes, because they have
four unrelated remediations and a single "signature chain invalid" leaves the
operator guessing which applies:

| Condition | Error code | Remediation the message names |
| --- | --- | --- |
| `publisher` is neither `d2b-official` nor a declared trusted publisher | `provider-signature-publisher-unregistered` | the option path `d2b.zones.<zone>.trustedPublishers.<publisher>` |
| `signatureId` names no key under a registered publisher | `provider-signature-id-unresolvable` | republish, or correct the catalog entry |
| The `.sig` file is not exactly 64 octets | `provider-signature-malformed` | fix the build; expected length `64` and the observed length are named |
| 64 well-formed octets that do not verify under the resolved key | `provider-signature-verification-failed` | re-sign the manifest |

None of the four is a warning.

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

#### 4.9.7 Anchored fd-relative resolution (normative)

Neither the resource compiler nor the launcher resolves a path in this layout by
building a string and calling `stat` followed by `open`.

Both open the pinned store path once with `O_PATH | O_DIRECTORY | O_CLOEXEC` and
perform every subsequent resolution with `openat2(2)` relative to that dirfd,
with `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`, and with a
path argument that is either a fixed literal from 4.9.2 or one already-validated
`bin/<name>` component from 4.9.3.

- `RESOLVE_BENEATH` makes escape from the anchor a kernel-enforced `EXDEV`.
- `RESOLVE_NO_SYMLINKS` refuses intermediate and trailing symlinks, so the
  regular-file requirements of 4.9.2 and 4.9.3 are enforced by the open itself
  rather than by a preceding `lstat` whose result could be stale.
- `RESOLVE_NO_MAGICLINKS` refuses `/proc/*/fd`-style jumps.

Measured, because it determines how the negative tests must be shaped: under
`BENEATH|NO_SYMLINKS|NO_MAGICLINKS` both an ordinary symlink and a symlink to a
magic link are refused with `ELOOP`, so `RESOLVE_NO_MAGICLINKS` is defense in
depth there and no test can attribute the refusal to it alone; under
`NO_MAGICLINKS` alone an ordinary symlink opens and a magic link is refused. With
no resolve restrictions the magic link opens, confirming both refusals come from
the flags. Consequently the anchored-resolution scenarios assert the requested
mask at the seam, while the 4.9.3a chain walk, which follows ordinary symlinks on
purpose, is the one place `RESOLVE_NO_MAGICLINKS` is load-bearing and its
negative test isolates the flag directly.

**Two handle modes are normative, because the compiler reads and the launcher
does not.** An `O_PATH` descriptor names a file without granting access to its
contents; `read(2)` on one returns `EBADF`, measured. A single `O_PATH` mode
therefore cannot serve the compiler, which must read the ELF prefix of 4.9.3 and
the bytes of every digest in 4.9.4:

| Caller | Open flags | Why |
| --- | --- | --- |
| Resource compiler | `O_RDONLY \| O_NONBLOCK \| O_CLOEXEC` | must read the file |
| Launcher | `O_PATH \| O_CLOEXEC` | never reads; `execveat` needs only a reference |

Both use the same `openat2` resolve set; the open mode is the only difference.

`O_NONBLOCK` is a denial-of-service guard, not an I/O style. Opening a FIFO for
reading blocks until a writer appears, and a compiler that blocks forever on a
hostile entry never finishes its build. Measured: on a regular file `O_NONBLOCK`
is ignored and the read returns `7f 45 4c 46` directly; on a FIFO the open
returns immediately and `fstat` reports `S_ISFIFO`, so the `S_ISREG` check
refuses it. A device node is the residual case, since `openat2` cannot filter by
file type and the refusal necessarily follows the open; `O_NONBLOCK` prevents the
hang, and a device node cannot normally exist in a store path because creating
one requires `CAP_MKNOD`, which the Nix builder lacks.

`execveat` also succeeds on an `O_RDONLY` descriptor, measured, so the launcher's
`O_PATH` is a least-authority choice rather than a kernel requirement.

**`O_CLOEXEC` is set on the anchor and on every child descriptor either caller
opens, in both modes, without exception.** This repository transfers descriptors
explicitly over `SCM_RIGHTS`; a descriptor surviving an exec it was not handed to
is an implicit transfer nobody authorised. A launched Provider binary must not
receive a handle to its own artifact directory.

Measured on Linux 7.0.10, because the interaction between `O_CLOEXEC` and
`execveat` is not self-evident:

```text
anchor fd=3 err=-
symlink open fd=-1 errno=Too many levels of symbolic links (expect refusal)
escape open fd=-1 errno=Invalid cross-device link (expect refusal)
subdir S_ISREG=0 (expect 0)
exe fd=5 err=-
exe S_ISREG=1 cloexec=1
PARENT passing fds anchor=3 exe=5 to execveat
CHILD inherited fds (excluding its own opendir fd 3):
  fd 0 -> /dev/null
  fd 1 -> pipe:[17462070]
  fd 2 -> pipe:[17462071]
CHILD /proc/self/exe -> /tmp/adr0050probe/tree/bin/child
```

`RESOLVE_NO_SYMLINKS` refuses a trailing symlink with `ELOOP` at the open;
`RESOLVE_BENEATH` refuses `bin/../../child` with `EXDEV`; `fstat` on an `O_PATH`
descriptor reports `S_ISREG = 0` for a directory; and
`execveat(fd, "", argv, envp, AT_EMPTY_PATH)` succeeds on an
`O_PATH | O_CLOEXEC` descriptor while the child inherits only fds 0, 1 and 2.

**Descriptor lifetime.** The descriptor MUST be open in the calling process at
the moment `execveat` is invoked. Close-on-exec is applied by the kernel as part
of a successful exec, after the image is resolved, so the descriptor is consumed
rather than leaked into the child. On a failed `execveat` the descriptor is still
open and still owned by the caller, so both it and the anchor are held in guards
that close on every path. Neither is ever passed to a child deliberately: the
child's descriptor table is exactly what its sandbox profile declares.

The child's `/proc/self/exe` resolves to the real store path rather than to
`/proc/self/fd/N`, so executing from a descriptor does not disturb a component
that reads its own image path.

Every opened descriptor is then `fstat(2)`ed **on the descriptor**, in both
modes, and refused unless `S_ISREG` holds. `fstat` works on an `O_PATH`
descriptor, which is what lets the launcher apply the same refusal without read
authority. In the compiler's `O_RDONLY` mode the 4.9.3 ELF prefix and every
4.9.4 digest are then read from that same descriptor; a path is never re-resolved
between the check and the read, so there is no check-to-use window.

The ordering is normative: open, `fstat`, refuse or proceed, then read or exec.
Reading before the `fstat` would read from a handle whose type is not established.

The launcher resolves `bin/<binaryRef>` through the same anchored `openat2` in
`O_PATH | O_CLOEXEC` mode, `fstat`s the descriptor for `S_ISREG`, and executes it
with `execveat(fd, "", argv, envp, AT_EMPTY_PATH)`. The program that runs is the
inode whose digest was verified.

**The sequence sits behind an injectable boundary.** `openat2`, `fstat`, and
`execveat` are reached only through these traits, so sequencing and errno mapping
are hermetically testable without a Nix store, a real symlink, or a real exec.
The two open modes are two methods rather than a flag, so a caller cannot request
authority it does not need:

```rust
pub trait AnchoredDir: Send + Sync {
    /// `O_RDONLY | O_NONBLOCK | O_CLOEXEC`, fstat-verified `S_ISREG`.
    type Readable: ReadableFile;
    /// `O_PATH | O_CLOEXEC`, fstat-verified `S_ISREG`.
    type Executable: ExecutableFile;

    fn open_readable(&self, path: LayoutPath) -> Result<Self::Readable, LayoutError>;
    fn open_executable(&self, path: LayoutPath) -> Result<Self::Executable, LayoutError>;
    fn entries(&self, dir: LayoutDir) -> Result<Vec<OsString>, LayoutError>;
}

pub trait ReadableFile: Send {
    fn len(&self) -> u64;
    fn read_prefix(&mut self, out: &mut [u8]) -> Result<usize, LayoutError>;
    fn read_to_digest(self) -> Result<ArtifactDigest, LayoutError>;
}

/// Exposes no read method, so the launcher cannot read a Provider image.
pub trait ExecutableFile: Send {}

pub trait ProcessLauncher: Send + Sync {
    type Executable: ExecutableFile;
    fn exec_from(&self, file: Self::Executable, argv: &Argv, envp: &Envp)
        -> Result<Infallible, LaunchError>;
}
```

Splitting the handle into two types makes the mode distinction a compile-time
fact rather than a comment: `ProcessLauncher` accepts nothing readable, and
`ExecutableFile` offers no way to read.

The production implementation MUST be the only place in the workspace naming
`openat2`, `execveat`, or `AT_EMPTY_PATH`. `LayoutError` carries the variants the
kernel actually produces at resolution time - `NotBeneath` (`EXDEV`),
`SymlinkRefused` (`ELOOP`), `NotRegular` (from `fstat`), `NotExecutable` (from the same `fstat` mode bits), `NotElf` (from the
compiler's prefix read), `NoDevice` (`ENXIO`, which an `O_RDONLY` open of a
socket returns), and `Absent` (`ENOENT` at open). `LaunchError` is a separate
enum carrying `FormatRejected` (`ENOEXEC`), `PermissionDenied` (`EACCES`), and `InterpreterUnresolvable`
(`ENOENT` returned by `execveat` rather than by the open). Two enums rather than
one is deliberate: an `ENOENT` at open and an `ENOENT` from exec mean different
things and map to different 4.9.8 codes, and a single enum would let them
collapse. The mapping onto the 4.9.8 codes is therefore a total match a test can
exhaust. `read_to_digest` and `exec_from` take the file **by value**, the same
single-use discipline ADR 0049 established for the mutation seal, so a verified
descriptor cannot be reused after its verification result was discarded.

This is the discipline ADR 0034 already binds for broker storage mutations
(anchored `openat2`, fd-relative walking, `O_CLOEXEC`, explicit fd transfer
only); it is reused here rather than forked.

Nix store immutability does not license skipping this. A store path is immutable
after registration, but neither the compiler nor the launcher runs at that
instant or can verify it: the artifact is an operator-supplied input that may be
locally built or substituted, `/nix/store` is a normal directory a privileged
process can write, and the launcher runs long after the compiler finished.

`openat2` requires Linux 5.6 and the supported floor is 6.9 (ADR 0008), so it is
unconditionally available and a kernel without it fails closed with no fallback.
`execveat` with `AT_EMPTY_PATH` on an `O_PATH` descriptor requires `/proc`
mounted in the callee's mount namespace to run a `#!` script, which is why 4.9.3
makes ELF a build-time requirement with its own code and scenario rather than a
caveat: the launcher never meets the case.

#### 4.9.8 Failure taxonomy (normative)

These codes join the section 13.4 table and inherit its rules unchanged: bounded
at 512 bytes, UTF-8 validated, control-character sanitized, and carrying no
secret, credential, path, or process data.

| Error code | Raised when | Message names |
| --- | --- | --- |
| `provider-artifact-output-ambiguous` | Multi-output package pinning its first output with no evidence of selection | artifact ID, declared output names, and both truthful remedies (4.9.1) |
| `provider-artifact-output-shape-unknown` | `outputs` present but not a non-empty list of strings | artifact ID |
| `provider-required-output-absent` | A required layout path is absent at resolution time: a 4.9.2 file at Phase 2, or `bin/<binaryRef>` when the launcher resolves it at Phase 3. Distinct from `provider-launch-format-rejected`, which follows a successful open | artifact ID, layout-relative path |
| `provider-required-output-not-regular` | A 4.9.2 path is a symlink, directory, or other non-regular type | artifact ID, layout-relative path, observed file type token |
| `provider-signature-publisher-unregistered` | See 4.9.6 | artifact ID, publisher, `d2b.zones.<zone>.trustedPublishers.<publisher>` |
| `provider-signature-id-unresolvable` | See 4.9.6 | artifact ID, publisher, signatureId |
| `provider-signature-malformed` | See 4.9.6 | artifact ID, expected length `64`, observed length |
| `provider-signature-verification-failed` | See 4.9.6 | artifact ID, publisher, signatureId |
| `provider-digest-mismatch` | Any pinned digest disagrees | artifact ID, which digest, expected value, actual value, the two disagreeing sources |
| `provider-manifest-not-canonical` | File octets are not their own canonical bytes | artifact ID, layout-relative path, byte offset of first divergence, expected and observed lengths, and the fixed remediation "re-emit with the toolkit canonical serializer; the usual cause is a trailing newline" |
| `provider-executable-name-invalid` | A `bin/` entry violates the 4.9.3 grammar | artifact ID, the rejected entry, the grammar |
| `provider-executable-set-empty` | `bin/` exists with no entries | artifact ID |
| `provider-executable-not-elf` | **Phase 2 only.** A `bin/` entry is not an `ET_EXEC`/`ET_DYN` ELF64 image, established by the compiler reading a bounded prefix | artifact ID, the entry, first four octets as hex, and the `d2b.lib.buildProviderElfShim` remedy |
| `provider-launch-format-rejected` | **Phase 3 only.** `execveat` refused the verified regular file: `ENOEXEC` for an invalid image, or `ENOENT` after a successful open, which can only be an unresolvable script interpreter | artifact ID, component ID, `binaryRef`, errno token. No content bytes are read, because the launcher holds `O_PATH` |
| `provider-launch-permission-denied` | **Phase 3 only.** `execveat` returned `EACCES`: a valid ELF the kernel will not execute, in practice a missing execute bit. Defense in depth behind `provider-executable-not-executable`, distinct because the remedy differs | artifact ID, component ID, `binaryRef`, errno token |
| `provider-component-execution-invalid` | **Component-level.** A non-bootstrap Provider component omits `binaryRef`, or claims `InProcessBootstrap` without being on the bootstrap-exception list | artifact ID, **component ID**, and the remedy |
| `provider-executable-declaration-inconsistent` | **Provider-level.** The §4.9.3 biconditional is violated: `executableDigests` empty while some component is `Launchable`, or non-empty while none is | artifact ID, which side failed, count of launchable components. **No component ID**, because the fact is Provider-wide |
| `provider-executable-not-executable` | **Phase 2 only.** A `bin/` entry is a valid ELF but `st_mode & 0o111 == 0` | artifact ID, the entry, observed mode as octal, and the remedy: set the execute bit in the install step |
| `provider-executable-not-regular` | A `bin/` entry is not a regular file | artifact ID, entry, observed file type token |
| `provider-executable-set-mismatch` | Key set and directory set differ | artifact ID, the symmetric difference and the side of each name |
| `provider-binary-ref-unresolved` | A `binaryRef` is not a key | artifact ID, component ID, the ref |

**Bounded safe representation.** Each named value is safe under section 13.4 for
a stated reason, rather than by assuming the class:

- `publisher` and `signatureId` are bounded catalog tokens and public artifact
  metadata, not credentials. Emitted whole.
- `d2b.zones.<zone>.trustedPublishers.<publisher>` is a **Nix option path**, not
  a filesystem path. It carries no host information and is the literal text the
  operator must type.
- Digests are fixed-width 71-byte `sha256:<64 hex>` tokens, so the bound holds by
  construction. Expected and actual are emitted **in full**: they cover
  world-readable store files and disclose nothing, and truncation would only make
  the comparison the operator needs harder.
- Layout-relative paths are fixed literals of 4.9.2. **`<out>` and any absolute
  path are never emitted**, which is the clause of 13.4 that binds.
- The canonical-JSON failure emits a byte offset and two lengths, never file
  content.
- Key material, manifest contents, config values, and store paths are never
  emitted by any code above.

One taxonomy serves both the Phase 2 build surface and any later status, audit,
or OTEL surface. A build message is an operator terminal and a status message is
a redaction boundary; two vocabularies is how a value that was safe in the first
reaches the second.

---

## 3. `ADR-046-resources-zone-control` section 14.10: replace one row, add one

### Phase 1 table: insert after the last Provider row

> | Provider artifact package resolves to one Nix output, carrying evidence of selection when the derivation has more than one (§4.9.1 predicate over `outputs`, `outputSpecified`, and `outputName`; `package.all` MUST NOT be read) - Provider only | Nix `assert` in `provider-catalog.nix` | eval error `provider-artifact-output-ambiguous` or `provider-artifact-output-shape-unknown`, naming the artifact ID and the declared output names |

### Phase 2 table: replace this row

> | Artifact catalog entry has required derivation outputs (manifest, config
> schema, executable) - Provider only | Resource compiler | build failure |

### With these nine rows

> | Required derivation paths present per section 4.9.2 (`share/d2b/provider/provider-manifest.json`, `provider-manifest.json.sig`, `config-schema.json`) - Provider only | Resource compiler | `provider-required-output-absent`, naming the layout-relative path |
> | Each of those three paths is a regular file, established by anchored `openat2` with `RESOLVE_NO_SYMLINKS` plus `fstat` on the descriptor (section 4.9.7) - Provider only | Resource compiler | `provider-required-output-not-regular`, naming the path and the observed file type |
> | `provider-manifest.json` and `config-schema.json` octets equal their own `d2b-cjson/v1` canonical bytes - Provider only | Resource compiler | `provider-manifest-not-canonical`, naming the file, the first divergent byte offset, and the remediation |
> | `bin/` entry set equals the signed manifest's `package.executableDigests` key set; each entry is a regular file - Provider only | Resource compiler | `provider-executable-set-mismatch` or `provider-executable-not-regular` |
> | Every `bin/` entry is an `ET_EXEC` or `ET_DYN` ELF64 image, established by a bounded 18-octet prefix read of the already-open descriptor (section 4.9.3) - Provider only | Resource compiler | `provider-executable-not-elf`, naming the entry and the first four octets as hex |
> | Every `bin/` directory entry name matches `^[a-z][a-z0-9-]*$`, is at most 64 bytes, and is valid UTF-8, checked as read from the directory (section 4.9.3) - Provider only | Resource compiler | `provider-executable-name-invalid`, naming the entry and the grammar |
> | `bin/` does not exist with zero entries; a Provider with no executables ships no `bin/` and an empty `package.executableDigests` - Provider only | Resource compiler | `provider-executable-set-empty` |
> | Every component `binaryRef` is a key of `package.executableDigests` - Provider only | Resource compiler | `provider-binary-ref-unresolved`, naming the component and the ref |
> | Each `bin/<name>` SHA-256 equals its `package.executableDigests` value - Provider only | Resource compiler | `provider-digest-mismatch`, naming the binary and both digest values in full |
> | Operator-authored catalog digests, manifest-declared digests, and compiler-recomputed digests agree pairwise (section 4.9.5) - Provider only | Resource compiler | `provider-digest-mismatch`, naming the two disagreeing sources and both values |

### Phase 2 table: the signature row keeps its wording and gains four codes

The existing row

> | Artifact manifest signature chain valid against installed trust store - Provider only | Resource compiler | build failure |

is replaced only in its failure-mode column:

> | Artifact manifest signature chain valid against installed trust store - Provider only | Resource compiler | one of `provider-signature-publisher-unregistered`, `provider-signature-id-unresolvable`, `provider-signature-malformed`, `provider-signature-verification-failed` (section 4.9.6) |

Its conformance scenario, `nix-build-manifest-signature-invalid`, is added in
section 4 below; it had none.

## 4. `ADR-046-resources-zone-control` section 15.8: add the missing scenarios

### Phase 1 - Nix eval tests: append

> | `nix-eval-provider-output-ambiguous` | A `type = "provider"` artifact whose package is a multi-output derivation pinning its first output with no evidence of selection fails eval naming the artifact ID and the declared output names; the same package with an output selected (`pkgs.foo.out`, `pkgs.foo.dev`) evaluates, and so does a non-first output selected on a raw-primop derivation. The residual is asserted explicitly: a raw-primop derivation whose **first** output is explicitly selected is **rejected**, being indistinguishable from the whole derivation, and the test pins that as intended rather than leaving it to be rediscovered as a bug |
> | `nix-eval-provider-output-shape-accepted` | A store-path-valued `types.package`, which `lib.types.package.check` accepts and the module system coerces to `outputs = ["out"]`, **evaluates successfully**; this pins the accepted behaviour so the predicate and the prose cannot drift apart again |
> | `nix-eval-provider-output-shape-unknown` | A value whose `outputs` is present but is not a non-empty list of strings fails eval with a module assertion rather than an eval trace |

### Phase 2 - Build tests: insert after `nix-build-manifest-digest-mismatch`

> | `nix-build-required-outputs-missing` | A Provider derivation missing any of `share/d2b/provider/provider-manifest.json`, `provider-manifest.json.sig`, or `config-schema.json` fails build naming the absent relative path |
> | `nix-build-manifest-signature-invalid` | Four distinct cases fail with four distinct codes: unregistered publisher, unresolvable `signatureId`, a `.sig` that is not exactly 64 octets, and 64 well-formed octets that do not verify |
> | `nix-build-required-output-not-regular` | Each of the three `share/d2b/provider/` paths, replaced in turn by a symlink (including one resolving inside the same output), **a symlink whose target is a procfs-style magic link such as `/proc/self/fd/<n>`**, a directory, a FIFO, a Unix socket, and a device node, fails build naming the path and the observed file type. The FIFO case must not hang, which is what `O_NONBLOCK` buys; the socket case is refused by `ENXIO` at open rather than by `fstat`, and both refusals are asserted distinctly. The magic-link case additionally asserts that the requested `open_how.resolve` mask carried `RESOLVE_NO_MAGICLINKS`, observable at the §4.9.7 seam, since with `RESOLVE_NO_SYMLINKS` also set the refusal alone cannot attribute itself to either flag |
> | `nix-build-manifest-not-canonical` | A manifest or config schema whose octets are not their own `d2b-cjson/v1` canonical bytes fails build naming the file and the first divergent byte offset; a trailing newline alone is sufficient to fail |
> | `nix-build-executable-set-mismatch` | `package.executableDigests` keys unequal to the `bin/` entry set fails build naming the symmetric difference |
> | `nix-build-executable-set-empty` | A derivation with a `bin/` directory containing no entries fails build, and is distinguished from a Provider that legitimately declares an empty `package.executableDigests` and ships no `bin/` at all, which succeeds |
> | `nix-build-executable-name-invalid` | A `bin/` entry whose name violates `^[a-z][a-z0-9-]*$`, exceeds 64 bytes, contains `/`, `.`, `..`, NUL, an ASCII control byte, or whitespace, begins with `-`, or is not valid UTF-8, fails build naming the entry; the name is checked as read from the directory, not only as declared in the manifest |
> | `nix-build-executable-not-regular-file` | A `bin/` entry that is a symlink, **a symlink to a procfs-style magic link**, a directory, FIFO, socket, or device node fails build naming the entry, with the same `RESOLVE_NO_MAGICLINKS` mask assertion at the §4.9.7 seam |
> | `nix-build-executable-not-elf` | A `bin/` entry that is a `#!` script, an empty file, a file shorter than the ELF header, an `ET_REL` object, or an `ET_CORE` image fails build naming the entry and the first four octets as hex, and the message names `d2b.lib.buildProviderElfShim` |
> | `nix-build-executable-not-executable` | A `bin/` entry that is a byte-identical valid `ET_DYN` ELF at mode `0644` fails build naming the entry and the observed mode, while the same bytes at `0755` succeed. The pair is asserted together, because every other §4.9.3 check admits both |
> | `nix-build-elf-shim-satisfies-the-layout` | A Provider whose entry point is a script, packaged through `d2b.lib.buildProviderElfShim`, produces a `bin/` entry passing the ELF, regular-file and name checks and launching its interpreter. A same-output relative chain (`bin/python3` to `bin/python3.13`) resolves and is baked in resolved form. **Eval negative:** `name` violating the §4.9.3 grammar. **Build negatives, chain shape:** the chain leaving the output, an absolute target, a `..` component, a chain exceeding 8 links, and **a link whose target is a procfs-style magic link**. The magic-link case is where `RESOLVE_NO_MAGICLINKS` is load-bearing rather than defense in depth, because this walk deliberately follows ordinary symlinks, so the test asserts the pair: an ordinary same-output link is followed while a magic link is refused under one mask. **Build negatives, chain terminus:** the chain ending at a `#!` wrapper script, an empty file, a file shorter than the ELF header, an `ET_REL` object, and a non-ELF regular file, each refused with the terminus named; and the helper's own output not being `ET_EXEC`/`ET_DYN`. **Runtime negatives:** the baked interpreter path replaced in turn by a symlink, a directory, a FIFO, and a Unix socket, each refused by the emitted shim rather than followed or opened, which proves the shim performs the `S_ISREG` check and not merely a symlink check. Every descriptor the shim opens carries `O_CLOEXEC` and is absent from the interpreter descriptor table |
> | `nix-build-component-execution-invalid` | A non-bootstrap Provider whose component descriptor omits `binaryRef` fails build with `provider-component-execution-invalid` naming the offending **component ID**; a bootstrap-exception Provider omitting it succeeds; a non-bootstrap Provider claiming `InProcessBootstrap` fails |
> | `nix-build-executable-declaration-inconsistent` | A Provider declaring an empty `executableDigests` while some component is `Launchable`, and one declaring a non-empty `executableDigests` while none is, both fail with `provider-executable-declaration-inconsistent`. The message names which side failed and **carries no component ID**, which is asserted |
> | `nix-build-manifest-binary-ref-wire-compatible` | A component descriptor authored with the flat `binaryRef` field of §4.3.3 parses unchanged, round-trips to identical bytes, and yields an unchanged `manifestDigest`; a `binaryRef` violating the §4.9.3 grammar is refused during deserialization rather than after |
> | `nix-build-binary-ref-unresolved` | A component descriptor `binaryRef` absent from `package.executableDigests` fails build naming the component and the ref |
> | `nix-build-executable-digest-mismatch` | A `bin/` file whose SHA-256 differs from its `package.executableDigests` value fails build naming the binary and both digests in full |
> | `nix-build-catalog-manifest-disagreement` | Operator-authored catalog digests, manifest-declared digests, and compiler-recomputed digests disagreeing on any pinned value fails build naming the two disagreeing sources and both digest values |
> | `nix-build-provider-error-redaction` | No failure message from any code in section 4.9.8 contains an absolute path, a `/nix/store` prefix, key material, manifest content, or a config value, and every message is within the section 13.4 512-byte bound |

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

> | Validation | All Phase 2 build tests in §15.8 (`nix-build-artifact-id-missing-from-catalog`, `nix-build-artifact-wrong-type-rejected`, `nix-build-duplicate-artifact-id`, `nix-build-artifact-store-path-absent-from-bundle`, `nix-build-artifact-store-path-absent-from-config`, `nix-build-config-schema-failure`, `nix-build-schema-digest-mismatch`, `nix-build-manifest-digest-mismatch`, `nix-build-required-outputs-missing`, `nix-build-required-output-not-regular`, `nix-build-manifest-signature-invalid`, `nix-build-manifest-not-canonical`, `nix-build-executable-set-mismatch`, `nix-build-executable-set-empty`, `nix-build-executable-name-invalid`, `nix-build-executable-not-regular-file`, `nix-build-executable-not-elf`, `nix-build-executable-not-executable`, `nix-build-elf-shim-satisfies-the-layout`, `nix-build-provider-error-redaction`, `nix-build-component-execution-invalid`, `nix-build-executable-declaration-inconsistent`, `nix-build-manifest-binary-ref-wire-compatible`, `nix-build-binary-ref-unresolved`, `nix-build-executable-digest-mismatch`, `nix-build-catalog-manifest-disagreement`, `nix-build-resourcetype-collision`, `nix-build-bundle-sorted`, `nix-build-content-hash-stable`, `nix-build-artifact-catalog-digest-anchored`, `nix-build-credential-ref-survives-build`, `nix-build-inline-secret-lint-warning`, `nix-build-inline-secret-strict-failure`) and the Phase 1 eval tests `nix-eval-provider-output-ambiguous`, `nix-eval-provider-output-shape-accepted`, and `nix-eval-provider-output-shape-unknown`. `nix-build-binary-ref-unresolved`, `nix-build-component-execution-invalid`, `nix-build-executable-declaration-inconsistent`, and `nix-build-manifest-binary-ref-wire-compatible` are blocked until `ComponentDescriptor` gains `ComponentExecution` with a validated `BinaryRef` and the flat-wire parse boundary (§4.9.3). `nix-build-elf-shim-satisfies-the-layout` is blocked until `d2b.lib.buildProviderElfShim` exists (§4.9.3a). The seventeen gated by neither are enumerated in §4.9.3 |

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

> | Validation | All Phase 3 runtime and cleanup tests in §15.8 (`nix-runtime-content-hash-integrity`, `nix-runtime-same-content-hash-noop`, `nix-runtime-zoneuid-mismatch-rejected`, `nix-runtime-zone-mismatch-rejected`, `nix-runtime-activation-nonblocking`, `nix-runtime-provider-config-invalid-continues`, `nix-runtime-launcher-anchored-resolution`, all `cleanup-*` and `rollback-*` tests) |

**One Phase 3 scenario is added, not just corrected.** The launcher contract of
sections 4.9.3 and 4.9.7 has no runtime conformance identity today, which is the
same defect on the Phase 3 side that this amendment fixes on the Phase 2 side.
Append to the §15.8 Phase 3 table:

> | `nix-runtime-launcher-anchored-resolution` | The launcher resolves a component program only as `bin/<binaryRef>` beneath the anchored artifact dirfd, with no fallback to `PATH` or to a manifest-supplied path. The resolved entry replaced by a symlink (including one whose target is a valid ELF inside the same output), **a symlink to a procfs-style magic link such as `/proc/self/fd/<n>`**, a directory, a FIFO, a socket, and a device node is refused before any process is created, by `RESOLVE_NO_SYMLINKS` and by the `S_ISREG` check respectively; the magic-link case also asserts `RESOLVE_NO_MAGICLINKS` in the requested mask at the §4.9.7 seam. An artifact path outside the anchor is refused as `NotBeneath`. A regular file that is not a valid image is refused by the `execveat` result (`provider-launch-format-rejected`), never by inspecting content, because the launcher holds `O_PATH` and cannot read. **EACCES is asserted as its own outcome:** a present valid ELF with the injected launcher returning `EACCES` surfaces as `LaunchError::PermissionDenied` and `provider-launch-permission-denied`, distinct from format and interpreter failures. **The two `ENOENT` sources are asserted separately and must not collapse:** with the entry absent, `openat2` returns `ENOENT`, surfacing as `LayoutError::Absent` and `provider-required-output-absent`; with the entry present and the injected launcher returning `ENOENT` from `execveat`, it surfaces as `LaunchError::InterpreterUnresolvable` and `provider-launch-format-rejected`. The second case is driven through the §4.9.7 `ProcessLauncher` double rather than a real script, so it assumes nothing about `/proc`, and the test asserts the two produce different enums and different codes. Every descriptor opened during resolution carries `O_CLOEXEC` and is absent from the launched process descriptor table |

It belongs to `016` rather than `015` because `015` is a build-time program and
the launcher is runtime. The symlink case in particular cannot be discharged at
Phase 2 alone: Phase 2 validated a different process's view of the tree at an
earlier time, which is exactly the check-to-use gap 4.9.7 exists to close.
Section 4.9.7's trait boundary is what makes this scenario hermetic rather than
requiring a privileged fixture that builds real symlinks and really execs.

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

### 6.3 `docs/specs/providers/ADR-046-provider-system-core.md`, section 5.2

This one is a **self-contradiction, not a naming drift**, and it must be fixed in
the same change or section 4.9.3 fails the bootstrap Provider it was written to
accommodate.

Section 5.2 declares two component descriptors, `host-controller` and
`user-controller`, and each carries:

```yaml
binaryRef: d2b-core-controller
```

Under 4.9.3, every `binaryRef` must be a key of `package.executableDigests`, and
under 4.9.2 `system-core` declares that map empty and ships no `bin/`.
`d2b-core-controller` is built by the separate `packages/d2b-core-controller`
derivation and is not in `system-core`'s artifact at all, so the reference cannot
resolve and never could.

**Delete the `binaryRef` line from both descriptors.** Do not repoint it at the
other derivation: a `binaryRef` naming a binary outside the artifact would defeat
the pinning that 4.9.4 and 4.9.5 exist to establish.

The deletion is consistent with the rest of the specification rather than a
special case. Section 11.3 step 5 already states that bootstrap exception
components (`system-core`, `system-minijail`) **skip component launch**, and
section 5.1 of the dossier itself states both handlers "run as handlers inside
the single fixed core-controller process". A descriptor from which no Process is
ever created has no program to name.

Add, immediately after the two descriptors:

> Neither descriptor carries a `binaryRef`. Both are bootstrap exception
> components (§11.3 step 5): the Zone runtime creates no Process from them, and
> their handlers execute in-process inside the fixed `d2b-core-controller`
> binary, which belongs to a different derivation and is not pinned by this
> Provider's artifact digests.

Apply the same review to `Provider/system-minijail`, per section 6.4 below,
which reaches the **opposite** conclusion and must not be inferred from this one.

### 6.4 `docs/specs/providers/ADR-046-provider-system-minijail.md`, section 4.1

The round-3 instruction to "apply the same review" to `system-minijail` was
wrong to leave open, because applying section 6.3's conclusion to it would be a
defect. `system-minijail` is a bootstrap exception for **Process creation** and
is **not** an in-process Provider: its dossier records
`Binary | d2b-provider-system-minijail (single executable)` and states that "the
controller is the only binary entry point". It therefore ships a `bin/` entry,
declares a non-empty `package.executableDigests`, and its component is
`Launchable`. Stripping its `binaryRef` the way section 6.3 strips
`system-core`'s would leave a Provider with a real binary and no way to name it.

The two bootstrap exceptions differ, and the amendment says so rather than
letting an implementer generalise:

| Provider | Has its own binary | `ComponentExecution` | `executableDigests` | `bin/` |
| --- | --- | --- | --- | --- |
| `system-core` | No; handlers link into `d2b-core-controller` | `InProcessBootstrap` | empty | absent |
| `system-minijail` | Yes; `d2b-provider-system-minijail` | `Launchable` | one entry | present |

Replace the `minijail-controller` inventory row

> | Binary | `d2b-provider-system-minijail` (single executable) |

with

> | `binaryRef` | `d2b-provider-system-minijail`; the component is `Launchable` (§4.9.3), so the derivation ships `bin/d2b-provider-system-minijail` and declares exactly one `package.executableDigests` entry keyed by that name |

Insert immediately after the paragraph beginning "There are no service, worker,
or separate component binaries in this Provider":

> `Provider/system-minijail` is a bootstrap exception for Process creation only:
> the Zone runtime starts its controller without a parent `Process` resource
> (§11.3 step 5). It is **not** an in-process Provider. Unlike
> `Provider/system-core`, whose handlers link into the `d2b-core-controller`
> binary from another derivation, `system-minijail` builds and ships its own
> executable, so its component descriptor carries a `binaryRef`, its artifact
> ships a `bin/` directory, and its `package.executableDigests` is non-empty.
> The `InProcessBootstrap` arm of `ComponentExecution` is admissible for this
> Provider but is not used by it.

No other edit to this dossier is required: it carries no `binaryRef` line to
correct and no manifest filename to rename, which is why the two dossiers need
different treatment rather than one shared instruction.

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
should cite where: append `(one Nix output, explicitly selected when the
derivation has more than one; layout fixed by ADR-046-resources-zone-control
section 4.9)`.

### 7.3 `ADR-046-nix-configuration`, "Validation" table

Add a row:

> | Provider artifact package resolves to one Nix output, explicitly selected when the derivation has more than one | Eval |

### 7.4 `ADR-046-security-and-threat-model`

The build-time signature verification entry names a single detection. Append the
four codes of section 4.9.6 so the threat model and the compiler agree on how
many distinguishable outcomes exist, and record that anchored `openat2`
resolution (section 4.9.7) is what closes the check-to-use window between
verifying a file and reading it.

### 7.5 `ADR-046-provider-model-and-packaging`, "Toolkit"

The Toolkit section enumerates what the official Rust toolkit provides and ends
with "Provider flake/project templates". Add the framework-owned Nix helper
alongside it, since it is the supported route the ELF rule of 4.9.3 depends on:

> - `d2b.lib.buildProviderElfShim`, the framework-owned builder that produces a
>   conforming ELF entry point for an interpreted Provider component
>   (`ADR-046-resources-zone-control` section 4.9.3a).

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
| `d2b.artifacts.<id>.package` typed `types.package` already enforces the cardinality at the one entry point (12.2, "inference, needs confirm or reject") | **Reject.** `types.package` pins one derivation per artifact ID; it does not pin one output per derivation, and `"${package}"` selecting the first output is exactly the case it misses. It is also weaker than it looks: `lib.types.package.check` accepts a bare store path, which the module system coerces through `lib.toDerivation`. The inference is superseded by the explicit §4.9.1 predicate |
| `ADR046-zone-control-015` stays blocked pending an amendment (19.7) | Closed on acceptance, with one carve-out: four scenarios stay blocked on the two obligations recorded in section 11, three on `ComponentExecution`/`BinaryRef` and one on the shim helper. `016` and `021` unblock, and `016` gains one new Phase 3 launcher scenario |
| Catalog names component and descriptor digests; contract names exported schema and service digests (12.4) | **Partly narrowed, not closed.** The executable digest was never part of the dispute: the catalog's singular value and the manifest's map are different objects, and section 2 states the derivation rule. The component/descriptor versus schema/service pair remains open and remains `ADR046-provider-002`'s |

## 10. Drift observed while drafting, recorded not fixed

Not in scope for this amendment; recorded so it is not lost.

| Fact | Where |
| --- | --- |
| `artifactId` maximum length is three different values: 128 characters in section 4.3.1, `maxArtifactIdLength = 64` in `nixos-modules/provider-catalog.nix`, and `MAX_ARTIFACT_ID_BYTES: usize = 63` in `packages/d2b-contracts/src/v3/provider.rs`. Two of the three are shipped code and they disagree with each other | 4.3.1 vs `provider-catalog.nix` vs `provider.rs` |
| `implementation-debt.md` 12.2 records "no Provider crate carries a `.nix` file". That is no longer true: six exist across four crates - `d2b-provider-network-local/nix/{default,artifacts,net-vm}.nix`, which are NixOS modules registering artifacts, and one `integration/*.nix` scenario declaration under each of `credential-entra`, `credential-managed-identity`, and `credential-secret-service`. None is a package derivation, so 12.2's conclusion survives, but its stated evidence does not | `implementation-debt.md` 12.2 |
| The root config schema is spelled three ways across the set: `config` (D075 and shipped `ProviderSpec::config`), `settingsSchemaDigest` in the `provider-catalog.json` example, and `configDigest` in the generated catalog shape. D075 and shipped code agree on `config` | D075, `ADR-046-nix-configuration`, `provider-catalog-shape.nix` |
| `SPIKE-05`, which would have exercised exactly this layout before it was specified, is recorded "Specified - not yet executed" and `proofs/provider-packaging-spike/` does not exist | `ADR-046-feasibility-and-spikes` |

## 11. Implementation obligation this amendment creates

Unlike section 10, these are **in scope and blocking**, so they are recorded
separately rather than as observed drift.

| Obligation | Owner | Blocks |
| --- | --- | --- |
| `ComponentDescriptor` in `packages/d2b-contracts/src/v3/provider.rs` gains a `ComponentExecution` field carrying a validated `BinaryRef` newtype, with `Deserialize` routed through the validating parser and the flat `binaryRef` wire mapping of §4.9.3. Not `Option<BinaryRef>`: `None` would mean both "not launchable" and "launchable, field omitted". The shipped struct declares `component_id`, `component_type`, `exported_resource_types`, `exported_methods`, `allowed_domains`, `cardinality`, `config_digest`, `dependencies`, `declares_state_volume`, `state_namespaces` and no `binary_ref`, while §4.3.3 defines `binaryRef` three times as normative. The `InProcessBootstrap` arm is admissible only for the closed bootstrap-exception list, per §6.3 | W5, alongside `ADR046-zone-control-015` | `nix-build-binary-ref-unresolved`, `nix-build-component-execution-invalid`, `nix-build-executable-declaration-inconsistent`, and `nix-build-manifest-binary-ref-wire-compatible`; §4.9.3 enumerates the seventeen gated by neither obligation |
| `d2b.lib.buildProviderElfShim` is implemented and exposed on the flake's existing `lib` output, per §4.9.3a. The flake already carries `lib = nixpkgs.lib.makeExtensible (_: { evalFixture = ...; })`, so this extends a surface rather than creating one. Framework-owned rather than a sibling flake because the framework is the party imposing the ELF rule | W5, alongside `ADR046-zone-control-015` | `nix-build-elf-shim-satisfies-the-layout`, which cannot be written before the helper exists; and it is what makes the ELF rule of §4.9.3 a supported path rather than only a prohibition |

The `ComponentDescriptor` drift predates this amendment. Both are recorded as
obligations rather than observed drift because §4.9.3 depends on them, and
carrying them in the observed-drift table would let T174 ship with the same shape
of hole it was blocked on.
