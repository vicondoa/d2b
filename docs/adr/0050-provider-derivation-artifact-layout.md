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
| `pkgs.openssl.out` | len 6 | 6 names | `out` | **true** |
| `lib.toDerivation "<store path>"` | **absent** | `["out"]` | `out` | absent |
| raw `derivation`, one output | **absent** | **absent** | `out` | absent |
| raw `derivation`, two outputs | len 2 | `["out","lib"]` | `out` | absent |
| raw `derivation`, `.lib` selected | len 2 | `["out","lib"]` | **`lib`** | absent |
| raw `derivation`, `.out` selected | len 2 | `["out","lib"]` | `out` | absent |

Two failures follow. First, `all` has length 6 on `pkgs.openssl.dev`, which
names exactly one output, so the predicate **rejects a correctly pinned single
output**. Second, `all` is *absent* on two shapes `types.package` accepts -
`lib.types.package.check "${pkgs.hello}"` returns `true`, and the module system
coerces such a string through `lib.toDerivation`, whose result carries
`["name","out","outPath","outputName","outputs","type"]` and no `all` - so
`builtins.length package.all` **throws**, replacing an actionable module
assertion with a raw eval trace.

`outputs` is the attribute to test, defaulted, together with `outputSpecified`
and `outputName`. `outputs` is absent on exactly one admitted shape, the raw
one-output primop, where defaulting to `["out"]` is correct.

**A store-path-valued package is accepted, not refused.** Round 2 caught a
contradiction between the first revision's predicate, which accepted it via the
one-output arm, and its scenario text, which claimed it failed. The measurement
settles it: `lib.toDerivation` yields `outputs = ["out"]`, one determinate
output. Nothing about the layout, the digests, or the signature chain is weaker
for it, so there is no security reason to refuse what the code already accepts.
The predicate is canon; the prose and the scenario were wrong and are corrected.

**`outputName` rescues the raw-primop selected output.** For a derivation built
with the raw primop, selecting `.lib` sets `outputName = "lib"` while the whole
derivation reports `outputName = "out"`, the first element of `outputs`. So
`outputName != head outputs` is a sound second witness of explicit selection for
exactly the shapes `outputSpecified` cannot cover.

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
  package = artifact.package;
  # `outputs` is absent on exactly one admitted shape, the raw one-output
  # primop, where "out" is the correct default. `all` is deliberately never
  # read: it is absent on store-path-coerced values, so reading it throws,
  # and it carries the whole derivation's output count on an explicitly
  # selected output, so reading it rejects a correctly pinned one.
  declaredOutputs = package.outputs or [ "out" ];
  shapeRecognised =
    builtins.isList declaredOutputs
    && declaredOutputs != [ ]
    && builtins.all builtins.isString declaredOutputs;
in
  # Ordered with `if` rather than `&&` so the guard visibly precedes every
  # partial operation. `builtins.head` and `builtins.length` are reached only
  # after `shapeRecognised` has established a non-empty list of strings.
  if !shapeRecognised then false
  else if builtins.length declaredOutputs == 1 then true
  else if (package.outputSpecified or false) == true then true
  else (package.outputName or null) != builtins.head declaredOutputs
```

The predicate is **total** over every shape `types.package` admits, which the
table in Context enumerates. It reads only `outputs`, `outputSpecified`, and
`outputName`, all defaulted, and every partial operation sits behind the
`shapeRecognised` guard. An unrecognised shape rejects rather than throws, so the
operator gets a module assertion instead of an eval trace.

What it accepts:

- a single-output derivation, including `pkgs.hello` and `pkgs.writeText`;
- a **store-path-valued** `types.package`, which `lib.types.package.check`
  admits and the module system coerces to `outputs = ["out"]`. This is what the
  code already did; round 2 corrected the prose that claimed otherwise;
- an explicitly selected output of a multi-output `mkDerivation`
  (`pkgs.foo.dev`, `pkgs.foo.out`), witnessed by `outputSpecified`;
- a non-first output selected on a raw-primop derivation, witnessed by
  `outputName != head outputs`.

What it rejects, and nothing else: a multi-output derivation whose pinned path is
the first output with no evidence anyone chose it, and a value declaring no
usable output list.

| Condition | Code | Message and remediation |
| --- | --- | --- |
| Multi-output, first output, no evidence of selection | `provider-artifact-output-ambiguous` | names the artifact ID and the declared output names, then gives the remedy for the case at hand: on a `stdenv.mkDerivation` derivation, select any output (`package = pkgs.<name>.out;`), which sets `outputSpecified`; on a raw `builtins.derivation`, selecting a **non-first** output already satisfies the check through the `outputName` witness, so the message says so, and only the remaining case - wanting the raw primop's **first** output - is directed to repackage with `stdenv.mkDerivation` |
| `outputs` present but not a non-empty list of strings | `provider-artifact-output-shape-unknown` | names the artifact ID and states that the value declares no usable output list; the remedy is to supply a derivation or a store path, not a hand-built attrset |

This is Phase 1, so it fails before any build starts. It earns its place even
though Phase 2 would also fail closed, because `nix-build-required-outputs-missing`
against an output the operator never meant to pin is a *misleading* diagnosis: it
points at the Provider's packaging rather than at the one-word omission in the
operator's own Nix.

Every path this ADR fixes is relative to the single store path that
`"${artifact.package}"` yields. That makes "the required outputs are present"
computable from the one value the catalog already records.

**The one residual case, stated exactly.** A raw-primop multi-output derivation
whose **first** output is explicitly selected (`rawMulti.out`) is
indistinguishable from the whole derivation: both report
`outputName = "out" = head outputs` and neither carries `outputSpecified`. It is
rejected. Both also coerce to the same store path, so the rejection is the same
answer the whole-derivation case gets, not a new one. The remedy the message
gives is truthful for it: repackage with `stdenv.mkDerivation`, or select a
non-first output. No Provider crate in the tree is built this way.

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

**Every `bin/` entry MUST be an ELF executable.** The check is a bounded read of
the first 18 octets from the already-open descriptor: `e_ident` must begin with
`\x7fELF`, `EI_CLASS` must be `ELFCLASS64`, `EI_DATA` must match the host byte
order, `EI_VERSION` must be 1, and `e_type` must be `ET_EXEC` or `ET_DYN`. No
parser is run over the file, nothing is mapped, and nothing is executed; the
compiler reads a fixed-length prefix of a descriptor it already holds and
compares constants. `ET_REL` and `ET_CORE` are refused, as is a `#!` script, an
empty file, and a file shorter than the header.

The rule exists for one concrete reason recorded in item 8: `execveat` with
`AT_EMPTY_PATH` on an `O_PATH` descriptor needs `/proc` mounted in the callee's
mount namespace to start a `#!` interpreter, and d2b sandbox profiles do not
promise one. Refusing non-ELF entries at Phase 2 means the launcher can never
meet a case whose behaviour depends on a mount that may not be there. The
alternative, discovering it at launch, converts a packaging error into a runtime
failure inside a sandbox.

A non-ELF entry fails the build with `provider-executable-not-elf`, naming the
entry and the observed first four octets rendered as hex. Four octets is the
bound: it distinguishes the common causes (`#!` shell script, an archive, a
text file) without echoing file content. The message names the remedy directly:
wrap the interpreter with `d2b.lib.buildProviderElfShim`, item 3a.

### 3a. The framework ships the ELF shim builder, because the framework imposed the rule

An ELF-only rule with no supported way to package an interpreted Provider is a
rule that gets worked around. The framework therefore owns the wrapper:

```nix
d2b.lib.buildProviderElfShim {
  inherit pkgs;
  name           = "d2b-provider-foo-controller";  # the bin/ entry name
  interpreterPkg = pkgs.python3;                   # the package output
  interpreterPath = "bin/python3";                 # relative, inside that output
  program        = ./controller.py;                # the script it runs
  extraArgs      = [ ];                            # optional, fixed at build time
}
```

The interpreter is given as **a package output plus a path relative to it**,
never as one interpolated absolute string. That split is what makes property 2
expressible: the output is the anchor the symlink walk may not leave, and the
relative path is what is walked beneath it. `"${pkgs.python3}/bin/python3"` would
erase that boundary the moment it became a single string.

The result is a derivation whose `$out/bin/<name>` is a **compiled** `ET_DYN`
image, not a `#!` line and not a `makeWrapper` shell script, which is why it
satisfies item 3 where those do not.

Seven properties are normative, because a wrapper that relaxes any of them
reintroduces the surface the layout closed:

1. The interpreter and the program are **resolved to canonical Nix store paths at
   build time** and baked in as string literals, split into a directory and a
   final component. The shim takes no argument that selects what to execute, and
   reads no environment variable to find it.
2. At build time the helper resolves the interpreter through a **bounded symlink
   chain that stays inside the same store output**, and requires the chain to end
   at a regular file whose first octets are an `ET_EXEC` or `ET_DYN` ELF64 image.
   Each link must be **relative**; an absolute target, any `..` component, a
   target that leaves the output, and a chain longer than 8 links are all
   refused. This is not a relaxation for convenience: it is what the most likely
   interpreter in the ecosystem actually looks like.

   ```text
   $ ls -l <python3-output>/bin/python3
   ... /bin/python3 -> python3.13
   $ head -c4 <python3-output>/bin/python3.13 | od -An -tx1
    7f 45 4c 46
   ```

   `pkgs.python3` reaches its real binary through a relative, same-output link,
   so a flat "no symlinks" rule would reject it. Confining the walk to one
   immutable output keeps the closure identity the shim depends on: every link
   traversed is inside the same content-addressed path, so nothing outside it is
   consulted and no cross-store canonicalization happens. There is no `realpath`
   call and no resolution against the ambient filesystem.

   **Wrapper-script interpreters are not supported, and the helper says so.** The
   same `bin/` directory contains entries like `idle3.13`, whose first octets are
   `23 21 2f 6e` - a `#!` line. If the chain ends at one of those, the build fails
   naming the entry and stating that a shebang wrapper cannot be used as a shim
   interpreter. The consumer's remedy is to name the real interpreter binary. This
   is a genuine limitation rather than an oversight: a wrapper script would put
   the framework back on the `execveat`-plus-`/proc` behaviour that item 3 exists
   to avoid.

   The **resolved same-output relative path** is what gets baked, not the
   authored one, so the shim executes the file that was verified rather than
   re-walking the chain at runtime.
3. The program, and the resolved interpreter, are **members of the shim
   derivation's runtime closure**, so Nix guarantees they are present wherever
   the shim is, and the closure digest of item 4 covers them. The program is
   verified to be a regular file by the same bounded walk; it is not required to
   be ELF, since the interpreter reads it.
4. At runtime the shim resolves the interpreter with the **same anchored fd
   discipline the framework uses on the shim itself**: open the baked interpreter
   directory `O_PATH | O_DIRECTORY | O_CLOEXEC`, `openat2` the final component
   with `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS` and
   `O_PATH | O_CLOEXEC`, `fstat` for `S_ISREG`, then
   `execveat(fd, "", argv, envp, AT_EMPTY_PATH)`. There is no `execve` on a
   concatenated string, no `PATH` search, no `execvp`, and no shell.
5. `argv[1..]` from the caller is forwarded after the fixed arguments, never
   interpreted as a program.
6. `name` is validated at eval against the item 3 grammar, so the helper cannot
   emit a `bin/` entry the compiler will later reject for its name.
7. The derivation self-checks its own output at build time: `readelf -h`
   confirms `ELFCLASS64` and an `e_type` of `ET_EXEC` or `ET_DYN`. The helper
   fails rather than producing something Phase 2 would refuse.

Property 7 is not new machinery. `flake.nix` already runs exactly this shape of
`postInstall` ELF assertion over the static guest helpers, checking `readelf -h`,
rejecting an unexpected program interpreter, and rejecting unexpected `NEEDED`
entries. The shim builder reuses that pattern rather than inventing a second one.

**Why this belongs in core rather than a sibling flake.** The bar this repository
sets is that every d2b user plausibly wants it and the framework cannot do the
right thing without it. Both hold here, and unusually so: the framework is the
party imposing the ELF requirement, so leaving the only compliant packaging route
to a third party would export a cost the framework created. It is also tiny, has
no runtime surface, and adds no dependency - it is a C file and a `stdenv.cc`
invocation.

**The framework still launches exactly one thing: the verified shim.** Items 7
and 8 are unchanged by this helper. The framework resolves
`<out>/bin/<binaryRef>` beneath the anchored artifact dirfd, verifies it, and
`execveat`s it. The shim's own subsequent exec of an interpreter is a *second*
exec performed by an already-launched, already-verified program inside its own
sandbox profile - the same position any Provider binary is in when it spawns a
child. The helper does not widen what the framework will launch.

**The exception is bounded to the framework-generated shim, and no further.**
Only a shim emitted by `d2b.lib.buildProviderElfShim` gets the build-time
verification and closure guarantees of properties 2 and 3. A hand-written
Provider binary that execs a sibling receives no exception, no verification, and
no special standing; it is bounded by its sandbox profile alone, exactly as
before. Nothing here licenses a general "Providers may exec things" rule.

**Why an immutable store path is not the string-path redirection it resembles.**
The objection is that baking `/nix/store/<hash>-python3/bin/python3` into a
binary is an unqualified absolute-path exec, and that a path is a name someone
else could rebind. Three facts make that not so here, and they are why property 4
is sufficient rather than merely comforting:

- **The path is a content address, not a name.** A store path's hash is derived
  from the derivation inputs. Changing the interpreter's content produces a
  *different* path. There is no version of "the same path, different content"
  that Nix will produce.
- **The path is in the shim's closure**, so Nix keeps it present and the garbage
  collector will not remove it while the shim is live. It cannot become dangling
  and then be recreated as something else.
- **Rebinding it requires writing to `/nix/store`**, which NixOS mounts read-only
  and which otherwise requires root. An attacker who can write the store can
  replace the shim, the Provider, and `d2bd` itself, so the shim's exec target is
  not the boundary that failed.

Property 4 does not rest on those three facts alone, which is the point: it
resolves the interpreter through the same `RESOLVE_NO_SYMLINKS` anchored open the
framework uses, so a symlink appearing at that path is refused at runtime rather
than trusted because the store is supposed to be immutable.

**What it does not do, stated exactly.** The interpreter is verified and executed
under fd discipline; the *program* the interpreter then reads is not, because the
interpreter opens it by path itself and no framework code sits in between. The
program is covered by build-time regular-file verification, canonical-path
baking, and closure membership, and by nothing stronger. Neither the interpreter
nor the program is named in `executableDigests`, because neither is a `bin/`
entry. That is the residual item 2 already records for the unpinned remainder,
narrowed rather than widened by this helper.

### 4. Digest preimages, stated exactly

Every value below renders as `sha256:<64 lowercase hex>`, which is what
`ArtifactDigest::parse` in `packages/d2b-contracts/src/v3/provider.rs` already
admits and what `provider-catalog.nix` already asserts at eval time.

| Value | Preimage |
| --- | --- |
| `package.executableDigests[<name>]` (manifest) | SHA-256 over the raw octets of `<out>/bin/<name>` |
| executable set digest (catalog) | `canonical_digest("d2b:v3:provider-executable-set", C)` where `C` is the `d2b-cjson/v1` serialization of the **whole `executableDigests` object**: every binary name as a key, bound to that binary's own `sha256:<hex>` value |
| `manifestDigest` | SHA-256 over the raw octets of `<out>/share/d2b/provider/provider-manifest.json` |
| `configSchemaDigest` | SHA-256 over the raw octets of `<out>/share/d2b/provider/config-schema.json` |
| package content digest | SHA-256 of the NAR serialization of the single output store path, as `nix hash path --type sha256 --base16` renders it |

Executable digests are raw file digests with no domain tag, because an ELF image
is not canonical JSON and there is nothing to canonicalize. The executable set
digest is domain-tagged because it is a JSON document, and it uses the shipped
`canonical_digest` helper unchanged: `SHA-256(domain_tag || 0x00 ||
canonical_bytes)`. `d2b:v3:provider-executable-set` is a new D101 domain tag and
registers alongside the nine already in use.

**Two functions in this workspace are named `canonical_digest` and they are not
the same function.** Pinned here because the distinction is easy to miss and
getting it wrong silently produces a digest nothing else can reproduce:

| Function | Formula | Payload |
| --- | --- | --- |
| `packages/d2b-contracts/src/v3/resource_schema.rs:518` | `SHA-256(domain_tag \|\| 0x00 \|\| canonical_bytes)` | `d2b-cjson/v1` canonical bytes |
| `packages/xtask/src/delivery/model.rs:591` | `SHA-256(domain \|\| payload_len_u64_be \|\| bytes)` | plain `serde_json::to_vec` output |

This ADR means the **first** one, which is the D101 contract digest that
`ADR-046-decision-register` D101 and `ADR-046-nix-configuration` both spell as
`SHA-256(domain_tag || 0x00 || canonical_bytes)`. The second belongs to the
delivery tooling's wave and panel records, hashes non-canonical JSON, and is not
a D101 digest at all. An implementer of `ADR046-zone-control-015` must import
`d2b_contracts::v3::canonical_digest` and no other.

**The set digest binds the map, not a summary of it.** Stated explicitly because
round 2 found the wording loose enough to read as a placeholder. The preimage is
the serialization of the object

```json
{"d2b-provider-foo-controller":"sha256:<64 hex>","d2b-provider-foo-service":"sha256:<64 hex>"}
```

so it covers every binary name **and** every per-binary digest, and the pairing
between them. It is not a digest of the name list, not a digest of the
concatenated digest values, and not a digest of a count or a tuple. Key order is
not a degree of freedom: `d2b-cjson/v1` is RFC 8785 JCS narrowed, so object keys
are sorted by code unit during serialization, and the same map therefore has one
digest regardless of the order any producer emitted it in. Renaming a binary,
replacing one binary's bytes, adding a binary, or removing one all change this
value; permuting the authoring order does not.

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
O_CLOEXEC`. The anchor is never read from, so `O_PATH` is the right authority for
it. Every subsequent resolution is `openat2(2)` relative to that dirfd with
`RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`, and the
path argument is a fixed literal from item 2 or a single already-validated
`bin/<name>` component from item 3. `RESOLVE_BENEATH` makes escape from the
anchor a kernel-enforced `EXDEV` rather than a check the caller could forget.
`RESOLVE_NO_SYMLINKS` refuses intermediate *and* trailing symlinks, so the
regular-file rules in items 2 and 3 are enforced by the open itself rather than
by a preceding `lstat` whose result could be stale.
`RESOLVE_NO_MAGICLINKS` refuses `/proc/*/fd`-style jumps.

**Two handle modes, because the compiler reads and the launcher does not.**
Round 3 stated a single `O_PATH` mode and then required the compiler to read the
ELF prefix and compute a digest from that descriptor. That is not possible, and
the correction is measured rather than argued:

```text
A O_PATH read -> n=-1 errno=Bad file descriptor
```

An `O_PATH` descriptor names a file without granting access to its contents, so
`read(2)` on one returns `EBADF`. The two callers therefore open in two different
modes, each the least authority that suffices:

| Caller | Open flags | Why |
| --- | --- | --- |
| Resource compiler | `O_RDONLY \| O_NONBLOCK \| O_CLOEXEC` | It must read: the ELF prefix of item 3 and the bytes of every digest in item 4 |
| Launcher | `O_PATH \| O_CLOEXEC` | It never reads the image; `execveat` needs only a reference |

Both use the same `openat2` resolve set. The mode is the only difference.

**`O_NONBLOCK` is a denial-of-service guard, not an I/O style.** Opening a FIFO
for reading blocks until a writer appears, and opening some device nodes blocks
or has side effects. A resource compiler that blocks forever on a hostile entry
in an artifact is a build that never finishes. `O_NONBLOCK` makes the open return
so the `fstat` can refuse it:

```text
B O_RDONLY|O_NONBLOCK regular: fd=5 S_ISREG=1 read=4 first4=7f 45 4c 46
B cloexec=1
C FIFO O_RDONLY|O_NONBLOCK -> fd=6 opened (did not block)
C fstat S_ISREG=0 S_ISFIFO=1 -> rejected by S_ISREG check
```

On a regular file `O_NONBLOCK` is ignored and the read returns the ELF magic
directly. On a FIFO the open returns immediately and `fstat` refuses it. The flag
costs nothing on the path that succeeds and closes the hang on the path that
fails.

A device node is the one case where the open itself is the exposure, since
`openat2` cannot filter by file type and the refusal necessarily comes after.
Two things bound it: `O_NONBLOCK` prevents the open from hanging, and a device
node cannot normally exist in a store path at all, because creating one needs
`CAP_MKNOD`, which the Nix builder does not have. The `S_ISREG` refusal is still
the authority; the second fact is why the case is not expected to arise.

**`O_CLOEXEC` on every descriptor, without exception.** The anchor dirfd and
every child descriptor either caller opens set `O_CLOEXEC` in `open_how.flags`,
in both modes. This is the same rule ADR 0034 states for storage locks, and it
exists for the same reason: this codebase transfers descriptors explicitly, over
`SCM_RIGHTS`, and a descriptor that survives an exec it was not handed to is an
implicit transfer nobody authorised. A Provider binary launched by item 7 must
not receive a writable-adjacent handle to its own artifact directory.

**Measured, because the interaction with `execveat` is not obvious.** The
concern is real: `O_CLOEXEC` on the very descriptor being executed sounds like it
should defeat the exec. It does not, and neither descriptor reaches the child:

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

Four facts follow, and each is load-bearing. `RESOLVE_NO_SYMLINKS` refuses a
trailing symlink with `ELOOP` at the open, so no `lstat` is needed to enforce
item 2 and item 3. `RESOLVE_BENEATH` refuses `bin/../../child` with `EXDEV`.
`fstat` works on an `O_PATH` descriptor and reports `S_ISREG = 0` for a
directory. And `execveat(fd, "", argv, envp, AT_EMPTY_PATH)` **succeeds on an
`O_PATH | O_CLOEXEC` descriptor** while the child inherits only fds 0, 1, and 2.

**Lifetime, stated so an implementer does not close it early.** The descriptor
must be open in the *calling* process at the moment `execveat` is invoked;
close-on-exec is applied by the kernel as part of a successful exec, after the
image has been resolved, so the fd is consumed rather than leaked. On a
*failed* `execveat` the descriptor is still open and still owned by the caller,
which must close it, so it is held in an RAII guard that closes on every path.
The anchor dirfd has the same lifetime and the same guarantee. Neither is ever
passed to a child deliberately, and item 6 of the launcher contract is that the
child's fd table is exactly what the sandbox profile declares.

One consequence worth recording: the child's `/proc/self/exe` resolves to the
real store path, not to `/proc/self/fd/N`, so `execveat`-from-descriptor does not
disturb any consumer that reads its own image path.

**Confirm after opening, not before.** Every opened descriptor is `fstat(2)`ed
**on the descriptor**, in both modes, and refused unless `S_ISREG` holds. `fstat`
works on an `O_PATH` descriptor, which is what lets the launcher apply the same
refusal without read authority. In the compiler's `O_RDONLY` mode the ELF prefix
of item 3 and every digest of item 4 are then read from *that same descriptor*,
never by reopening the path. There is no window between the check and the use
because there is no second resolution.

**Ordering is normative:** open, `fstat`, refuse or proceed, then read or exec.
Reading before the `fstat` would mean reading from a handle whose type is not yet
established.

**Launch.** The launcher resolves `bin/<binaryRef>` through the same anchored
`openat2` in `O_PATH | O_CLOEXEC` mode, `fstat`s the descriptor for `S_ISREG`,
and executes it with `execveat(fd, "", argv, envp, AT_EMPTY_PATH)`. The program
that runs is the inode the digest was computed over, with no path re-traversal in
between.

`execveat` also succeeds on an `O_RDONLY` descriptor, measured, so the launcher's
use of `O_PATH` is a least-authority choice rather than a kernel requirement: the
launcher has no reason to hold a readable handle to a Provider image, so it does
not take one.

### 8a. The syscall sequence sits behind an injectable boundary

`openat2`, `fstat`, and `execveat` are reached only through two traits, so the
sequencing and the error mapping are testable without a Nix store, a real
symlink, or a real exec:

```rust
/// A directory the caller has already anchored. Implementations resolve
/// only single, pre-validated components beneath the anchor.
///
/// The two open modes of item 8 are two methods rather than a flag
/// argument, so a caller cannot ask for a readable handle it does not need,
/// and a launcher cannot accidentally receive one.
pub trait AnchoredDir: Send + Sync {
    /// `O_RDONLY | O_NONBLOCK | O_CLOEXEC`, fstat-verified `S_ISREG`.
    type Readable: ReadableFile;
    /// `O_PATH | O_CLOEXEC`, fstat-verified `S_ISREG`.
    type Executable: ExecutableFile;

    /// Resolve one layout-relative path to a readable regular file, refusing
    /// symlinks, magic links, escapes, and non-regular types.
    fn open_readable(&self, path: LayoutPath) -> Result<Self::Readable, LayoutError>;
    /// Resolve one layout-relative path to an executable reference, with the
    /// same refusals and no read authority.
    fn open_executable(&self, path: LayoutPath) -> Result<Self::Executable, LayoutError>;
    /// Enumerate the entries of a closed subdirectory.
    fn entries(&self, dir: LayoutDir) -> Result<Vec<OsString>, LayoutError>;
}

/// An opened regular file the compiler may read. Never re-resolved.
pub trait ReadableFile: Send {
    fn len(&self) -> u64;
    fn read_prefix(&mut self, out: &mut [u8]) -> Result<usize, LayoutError>;
    fn read_to_digest(self) -> Result<ArtifactDigest, LayoutError>;
}

/// An `O_PATH` reference the launcher may execute and cannot read.
pub trait ExecutableFile: Send {}

/// Launching a program from an already-opened, already-verified reference.
pub trait ProcessLauncher: Send + Sync {
    type Executable: ExecutableFile;
    fn exec_from(&self, file: Self::Executable, argv: &Argv, envp: &Envp)
        -> Result<Infallible, LaunchError>;
}
```

Splitting the handle into two types is what makes the mode distinction a
compile-time fact rather than a comment. `ExecutableFile` exposes no read method,
so the launcher cannot read a Provider image even by mistake, and
`ProcessLauncher` accepts nothing else, so a readable handle cannot be executed
without first being reopened in the other mode.

The production implementation is the only place in the workspace that names
`openat2`, `execveat`, or `AT_EMPTY_PATH`, which makes "is the sequence correct"
a question about one module rather than about every caller. `LayoutError` carries
the distinguished variants the kernel actually produces - `NotBeneath` for
`EXDEV`, `SymlinkRefused` for `ELOOP`, `NotRegular` from the `fstat`, `NotElf`
from the compiler's prefix read, `NoDevice` for the `ENXIO` an `O_RDONLY` open of
a socket returns, and `Absent` for `ENOENT` at open. `LaunchError` is separate
and carries `FormatRejected` for `ENOEXEC` and `InterpreterUnresolvable` for an
`ENOENT` returned by `execveat` rather than by the open. Keeping them in two
enums is what stops an `ENOENT` from the open and an `ENOENT` from the exec
collapsing into one variant, since they mean different things and map to
different item 9 codes.

The test implementation is an in-memory tree that can be told to present an entry
as a symlink, a directory, a FIFO, a socket, a short file, a non-ELF file, or an
escape, and a launcher that records what it was handed, or returns a chosen
errno, instead of execing. That is what makes
`nix-build-required-output-not-regular`, `nix-build-executable-not-elf`, and the
Phase 3 launcher scenario hermetic rather than requiring a privileged fixture.

Taking the handle **by value** in `read_to_digest` and `exec_from` is the
same single-use discipline ADR 0049 established for the mutation seal: a
descriptor that was verified and then digested cannot be handed to the launcher
a second time after the verification result has been discarded.

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
fails closed. And `execveat` with `AT_EMPTY_PATH` on an `O_PATH` fd cannot start
a `#!` script, because the interpreter would have to reopen the image through
`/proc/self/fd/N` and an `O_PATH` descriptor grants no read access. Measured:

```text
plain.txt      S_ISREG=1 execveat errno=Exec format error
empty.bin      S_ISREG=1 execveat errno=Exec format error
script.sh      S_ISREG=1 execveat errno=No such file or directory
child          EXECUTED (exit 0)
```

Three consequences, all used elsewhere in this record. Item 3's Phase 2 ELF rule
is what keeps the launcher from ever meeting the script case, which presents as a
confusing `ENOENT` rather than a format error. The launcher needs **no content
inspection** to refuse a bad image, since the kernel reports `ENOEXEC` for a
regular file that is not a valid image, which is what makes item 9's runtime code
implementable from an `O_PATH` descriptor. And an `ENOENT` returned by
`execveat` is unambiguous in context: the open already succeeded, so it can only
mean an unresolvable script interpreter, never an absent artifact.

### 9. One bounded, actionable failure taxonomy

Section 13.4 already binds the shape: stable kebab-case codes, messages bounded
at 512 bytes, UTF-8 validated, control-character sanitised, and carrying no
secret, credential, path, or process data. These codes join that table.

| Code | Raised when | Names |
| --- | --- | --- |
| `provider-artifact-output-ambiguous` | Multi-output package pinning its first output with no evidence of selection | artifact ID, declared output names, and both truthful remedies |
| `provider-artifact-output-shape-unknown` | `outputs` present but not a non-empty list of strings | artifact ID |
| `provider-required-output-absent` | A required layout path is absent at resolution time: a file from item 2 at Phase 2, or `bin/<binaryRef>` when the launcher resolves it at Phase 3. Distinct from `provider-launch-format-rejected`, which follows a successful open | artifact ID, layout-relative path |
| `provider-required-output-not-regular` | A layout path is a symlink, directory, or other non-regular type | artifact ID, layout-relative path, observed file type token |
| `provider-executable-not-elf` | **Phase 2 only.** A `bin/` entry is not an `ET_EXEC`/`ET_DYN` ELF64 image, established by the compiler reading a bounded prefix | artifact ID, the entry, first four octets as hex, and the `d2b.lib.buildProviderElfShim` remedy |
| `provider-launch-format-rejected` | **Phase 3 only.** `execveat` refused the verified regular file: `ENOEXEC` for an invalid image, or `ENOENT` after a successful open, which can only be an unresolvable script interpreter | artifact ID, component ID, `binaryRef`, and the errno token. No content bytes are read or reported, because the launcher holds `O_PATH` |
| `provider-component-execution-invalid` | A non-bootstrap Provider omitted `binaryRef`, or the empty-`executableDigests` biconditional of item 14 is violated | artifact ID, component ID |
| `provider-signature-publisher-unregistered` | `publisher` is neither `d2b-official` nor a declared trusted publisher | artifact ID, publisher, the option path `d2b.zones.<zone>.trustedPublishers.<publisher>` |
| `provider-signature-id-unresolvable` | `signatureId` names no key under a registered publisher | artifact ID, publisher, signatureId |
| `provider-signature-malformed` | The `.sig` file is not exactly 64 octets | artifact ID, expected length `64`, observed length |
| `provider-signature-verification-failed` | 64 well-formed octets that do not verify under the resolved key | artifact ID, publisher, signatureId |
| `provider-digest-mismatch` | Any pinned digest disagrees | artifact ID, which digest, expected value, actual value, and the two disagreeing sources |
| `provider-manifest-not-canonical` | File octets are not their own canonical bytes | artifact ID, layout-relative path, byte offset of first divergence, expected and observed lengths |
| `provider-executable-name-invalid` | A `bin/` entry violates the item 3 grammar | artifact ID, the rejected entry, the grammar |
| `provider-executable-set-empty` | `bin/` exists but has no entries | artifact ID |
| `provider-executable-not-regular` | A `bin/` entry is a symlink, directory, FIFO, socket, or device node | artifact ID, the entry, observed file type token |
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
| `nix-eval-provider-output-ambiguous` | 1 | A `type = "provider"` artifact whose package is a multi-output derivation pinning its first output with no evidence of selection fails eval naming the artifact ID and the declared output names; the same package with an output selected (`pkgs.foo.out`, `pkgs.foo.dev`) evaluates, and so does a non-first output selected on a raw-primop derivation. The residual is asserted explicitly: a raw-primop derivation whose **first** output is explicitly selected is **rejected**, because it is indistinguishable from the whole derivation, and the test pins that as intended behaviour rather than leaving it to be rediscovered as a bug |
| `nix-eval-provider-output-shape-accepted` | 1 | A store-path-valued `types.package`, which `lib.types.package.check` accepts and the module system coerces to `outputs = ["out"]`, **evaluates successfully**; this pins the accepted behaviour so the predicate and the prose cannot drift apart again |
| `nix-eval-provider-output-shape-unknown` | 1 | A value whose `outputs` is present but is not a non-empty list of strings fails eval with a module assertion rather than an eval trace |
| `nix-build-required-outputs-missing` | 2 | A Provider derivation missing any of `share/d2b/provider/provider-manifest.json`, `provider-manifest.json.sig`, or `config-schema.json` fails build naming the absent layout-relative path |
| `nix-build-required-output-not-regular` | 2 | Each of the three `share/d2b/provider/` paths, replaced in turn by a symlink (including one resolving inside the same output), a directory, a FIFO, a Unix socket, and a device node, fails build naming the path and the observed file type. The FIFO case must not hang, which is what `O_NONBLOCK` buys; the socket case is refused by `ENXIO` at open rather than by `fstat`, and both refusals are asserted distinctly |
| `nix-build-executable-not-elf` | 2 | A `bin/` entry that is a `#!` script, an empty file, a file shorter than the ELF header, an `ET_REL` object, or an `ET_CORE` image fails build naming the entry and the first four octets as hex, and the message names `d2b.lib.buildProviderElfShim` |
| `nix-build-elf-shim-satisfies-the-layout` | 2 | A Provider whose entry point is a script, packaged through `d2b.lib.buildProviderElfShim`, produces a `bin/` entry that passes the ELF, regular-file, and name checks and launches its interpreter. A same-output relative chain (`bin/python3` to `bin/python3.13`) resolves and is baked in its resolved form. **Eval negative:** `name` violating the item 3 grammar. **Build negatives, chain shape:** the chain leaving the output, an absolute target, a `..` component, and a chain exceeding 8 links. **Build negatives, chain terminus:** the chain ending at a `#!` wrapper script, an empty file, a file shorter than the ELF header, an `ET_REL` object, and a non-ELF regular file, each refused with the terminus named; and the helper's own output not being `ET_EXEC`/`ET_DYN`. **Runtime negatives:** the baked interpreter path replaced in turn by a symlink, a directory, a FIFO, and a Unix socket, each refused by the emitted shim rather than followed or opened, which is what proves the shim performs the `S_ISREG` check and not merely a symlink check. Every descriptor the shim opens to resolve the interpreter carries `O_CLOEXEC` and is absent from the interpreter's descriptor table |
| `nix-build-component-execution-invalid` | 2 | A non-bootstrap Provider whose component descriptor omits `binaryRef` fails build; a bootstrap-exception Provider omitting it succeeds; a Provider declaring an empty `executableDigests` while a component still names a `binaryRef`, and the converse, both fail |
| `nix-build-manifest-binary-ref-wire-compatible` | 2 | A component descriptor authored with the flat `binaryRef` field of section 4.3.3 parses unchanged, round-trips to identical bytes, and yields an unchanged `manifestDigest`; a `binaryRef` violating the item 3 grammar is refused during deserialization rather than after |
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

`ADR046-zone-control-016` additionally **gains** one Phase 3 scenario. The
launcher contract of items 7 and 8 has no runtime conformance identity, which is
the same defect this ADR was written to fix on the Phase 2 side, so it is not
left for a later record:

| Scenario | Phase | Assertion |
| --- | --- | --- |
| `nix-runtime-launcher-anchored-resolution` | 3 | The launcher resolves a component program only as `bin/<binaryRef>` beneath the anchored artifact dirfd, with no fallback to `PATH` or to a manifest-supplied path. The resolved entry replaced by a symlink (including one whose target is a valid ELF inside the same output), by a directory, by a FIFO, by a socket, and by a device node is refused before any process is created, by `RESOLVE_NO_SYMLINKS` and by the `S_ISREG` check respectively. An artifact path outside the anchor is refused as `NotBeneath`. A regular file that is not a valid image is refused by the `execveat` result, not by inspecting its contents, because the launcher holds `O_PATH` and cannot read. **The two `ENOENT` sources are asserted separately and must not collapse:** with the entry absent, `openat2` returns `ENOENT`, which surfaces as `LayoutError::Absent` and `provider-required-output-absent`; with the entry present and the injected launcher returning `ENOENT` from `execveat`, it surfaces as `LaunchError::InterpreterUnresolvable` and `provider-launch-format-rejected`. The test drives the second through the item 8a `ProcessLauncher` double rather than a real script, so it needs no `/proc` assumption, and it asserts the two produce different enums and different codes. Every descriptor opened during resolution carries `O_CLOEXEC` and is absent from the launched process's descriptor table |

Cited in `016`'s validation field alongside the corrected Phase 3 names. It
belongs to `016` rather than to `015` because `015` is a build-time program and
the launcher is runtime; the symlink case in particular cannot be caught at
Phase 2 alone, because Phase 2 validated a different process's view of the tree
at an earlier time.

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

### 14. `BinaryRef` is a validated newtype, and launchability is modelled, not optional

Item 3's `binaryRef` check cannot be written against the contract as it stands.
The shipped `ComponentDescriptor` in `packages/d2b-contracts/src/v3/provider.rs`
carries `component_id`, `component_type`, `exported_resource_types`,
`exported_methods`, `allowed_domains`, `cardinality`, `config_digest`,
`dependencies`, `declares_state_volume`, and `state_namespaces`. It has **no
`binary_ref` field at all**, while section 4.3.3 defines `binaryRef` three times
as a normative descriptor field.

That drift predates this ADR, but item 3 depends on it, so this ADR fixes the
shape the field must take rather than leaving the next implementer to choose.

**The reference is a validated newtype, not a string.**

```rust
/// A `^[a-z][a-z0-9-]*$` component binary name, at most 64 bytes.
///
/// Parsing is the only constructor, so a value of this type is a name the
/// item 3 grammar already admitted: no `/`, no `.` or `..`, no NUL, no ASCII
/// control byte, no whitespace, no leading `-`, valid UTF-8. Nothing in the
/// launcher re-checks it, because nothing can construct one that failed.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct BinaryRef(BoundedToken);

impl BinaryRef {
    pub fn parse(value: impl Into<String>) -> Result<Self, ProviderContractError> { /* ... */ }
}

// Deserialization goes through the validating parser, never through a
// derived impl, so an unvalidated `BinaryRef` cannot exist even transiently.
// This mirrors `ArtifactId` in the same module exactly.
impl<'de> Deserialize<'de> for BinaryRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

redacted_debug!(BinaryRef);
string_schema!(BinaryRef, 1, MAX_BINARY_REF_BYTES);
```

The hand-written `Deserialize` is not a stylistic preference: `#[serde(transparent)]`
on the derive would accept any string, so the grammar of item 3 would hold only
for values constructed in Rust and not for values parsed from a signed manifest,
which is the only path that matters. `ArtifactId` in the same file already uses
this exact shape, so this is the module's existing convention rather than a new
one.

It is a distinct type from `ArtifactId` and `ComponentId` even though the three
grammars coincide today, because they are three different namespaces and a
coincidence of syntax is not a licence to substitute one for another. The
newtype is what lets `AnchoredDir::open_readable` and `open_executable` accept a
component name without
a traversal check at the call site: the type is the check.

**Launchability is an enum, so there is no contradictory state.** A bare
`Option<BinaryRef>` would admit two readings of `None` - "this component is not
launchable" and "this component is launchable but the manifest omitted the
field" - and the second is exactly the hole item 3 exists to close. The
distinction is therefore made in the type:

```rust
pub enum ComponentExecution {
    /// The Zone runtime creates a Process from this component and launches
    /// exactly `<out>/bin/<binary_ref>`.
    Launchable { binary_ref: BinaryRef },
    /// The component's handlers execute in-process inside a binary belonging
    /// to a different derivation. No Process is created (section 11.3 step 5).
    InProcessBootstrap,
}
```

**The wire representation stays the flat `binaryRef` field, and the schema is
not versioned.** The enum is an in-memory invariant, not a new JSON shape. No
serde enum representation is used - not `tag`, not `untagged`, not `adjacent` -
because every one of them would change the manifest bytes that section 4.3.3
already specifies and that every Provider dossier already writes:

```rust
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Wire {
    // ... the other descriptor fields ...
    #[serde(default)]
    binary_ref: Option<BinaryRef>,
}

// Present -> Launchable. Absent -> InProcessBootstrap. Exactly one mapping,
// applied once, at the parse boundary.
let execution = match wire.binary_ref {
    Some(binary_ref) => ComponentExecution::Launchable { binary_ref },
    None => ComponentExecution::InProcessBootstrap,
};
```

Serialization is the inverse: `Launchable` emits `binaryRef`,
`InProcessBootstrap` omits it via `skip_serializing_if`. A descriptor therefore
round-trips to the same bytes it was parsed from, existing manifests parse
unchanged, and `manifestDigest` is unaffected. **No `schemaVersion` bump and no
`apiVersion` bump is required**, which is the point of choosing this
representation over a tagged one.

The apparent ambiguity has not been reintroduced, it has been *relocated to the
only place it can be resolved*. On the wire a field is present or absent and
nothing more can be said; whether an absent field is legitimate is a fact about
the Provider, not about the JSON. So the parse boundary maps absence to a named
variant, and admission decides whether that variant was allowed. In memory,
after admission, there is exactly one representation of each state and no
`Option` for a caller to misread.

`ComponentDescriptor` holds one `ComponentExecution`, not an optional ref, so
"launchable" and "has a binary name" are the same fact and cannot disagree.

Two admission rules keep the bootstrap arm from becoming a general escape hatch:

1. `InProcessBootstrap` is admissible only for a Provider on the closed
   bootstrap-exception list (`system-core`, `system-minijail`). Any other
   Provider declaring it - that is, omitting `binaryRef` - is refused at
   manifest admission with `provider-component-execution-invalid`.
2. A Provider whose components are all `InProcessBootstrap` MUST declare an
   empty `package.executableDigests` and ship no `bin/`, and conversely. This is
   the same biconditional item 2 states, now checkable on the contract side
   rather than only against the filesystem. A violation raises the same code.

**W5 implementation obligations, and exactly which scenarios each gates.** Stated
as explicit lists rather than totals, because a count is the thing that goes
stale first.

| Obligation | Gates |
| --- | --- |
| `ComponentDescriptor` gains `ComponentExecution` with `BinaryRef`, the flat-wire parse boundary, and the validating `Deserialize` | `nix-build-binary-ref-unresolved`, `nix-build-component-execution-invalid`, `nix-build-manifest-binary-ref-wire-compatible` |
| `d2b.lib.buildProviderElfShim` exists, per item 3a | `nix-build-elf-shim-satisfies-the-layout` |

Four scenarios are gated. The sixteen that are not, named so the list can be
checked rather than trusted: `nix-eval-provider-output-ambiguous`,
`nix-eval-provider-output-shape-accepted`,
`nix-eval-provider-output-shape-unknown`, `nix-build-required-outputs-missing`,
`nix-build-required-output-not-regular`, `nix-build-executable-not-elf`,
`nix-build-manifest-signature-invalid`, `nix-build-manifest-not-canonical`,
`nix-build-executable-set-mismatch`, `nix-build-executable-set-empty`,
`nix-build-executable-name-invalid`, `nix-build-executable-not-regular-file`,
`nix-build-executable-digest-mismatch`,
`nix-build-catalog-manifest-disagreement`, `nix-build-provider-error-redaction`,
and the Phase 3 `nix-runtime-launcher-anchored-resolution`.

That is 20 scenarios in total: 19 in item 11 plus the Phase 3 one in item 12.
Neither obligation is created by this ADR; the `ComponentDescriptor` drift
predates it, and the helper is the earned path item 3a commits to. The amendment
records the same split so a slice can start on the sixteen without waiting.

## Consequences

**A Provider package becomes a checkable object rather than an assertion.**
`ADR046-zone-control-015` can now write the Phase 2 required-outputs check
against fixed paths, and every one of the nineteen scenarios in item 11 is a
filesystem or digest comparison a machine evaluates, with a twentieth in
item 12 covering the launcher at Phase 3. The blocking status recorded
in implementation-debt 19.7 lifts for `015`, and with it for `016` and `021` -
with four exceptions, which item 14 records as blocked on a contract field and a
framework helper that do not exist yet.

**Multi-output Provider packaging is permitted but must be said out loud.** This
is where panel round 1 moved the decision, and round 2 widened it further. A
Provider may ship a multi-output derivation; the operator names the output that
carries the Provider (`package = pkgs.foo.out;`), and a non-first output selected
on a raw-primop derivation is accepted through `outputName` without needing
`outputSpecified`. One case remains refused that a human would call correct: the
*first* output explicitly selected on a raw-primop derivation, which is
indistinguishable from no selection and pins the same path either way.

**Requiring ELF costs something, and the framework pays part of it.** A Provider
whose component entry point is a `#!` wrapper is refused at Phase 2. Round 3
made that cost survivable rather than merely stated: item 3a commits the
framework to `d2b.lib.buildProviderElfShim`, so the supported route for an
interpreted Provider is one helper call rather than a bespoke C wrapper per
publisher. The residual cost is real but bounded - a Provider author must know
the helper exists, and the error message names it - and the alternative is a
launcher whose success depends on whether `/proc` happens to be mounted in the
callee's namespace, which is a failure mode that appears only in production and
only in some profiles.

**Owning the shim builder is new framework surface.** It is a `.nix` file, a
small C source, and a `stdenv.cc` invocation, with no runtime component and no
new dependency, but it is still a public API the framework must keep working.
The bar was met deliberately: the framework is the party imposing the ELF rule,
so exporting the only compliant packaging route to a sibling flake would export
a cost the framework created.

**The trait boundary is indirection that has to earn its keep.** Item 8a puts
three syscalls behind two traits, which is more machinery than calling them
directly. It buys hermetic coverage of the sequencing and the errno mapping
without a privileged fixture, and it makes "does anything else in the workspace
call `execveat`" a greppable question with one answer.

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

**Eighteen error codes is a real surface.** Item 9 adds eighteen rows to section
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

**Testing `outputName` against the first element of `outputs`, as the *only*
witness of selection.** Round 2 adopted this as a *second* witness, after
`outputSpecified`, which is where it belongs. As the only witness it is wrong:
`pkgs.openssl.bin` selects the first output explicitly and would be refused. As a
fallback it is sound, because it fires only where `outputSpecified` is absent,
which is exactly the raw-primop shapes nixpkgs conventions do not reach.

**Refusing a store-path-valued `types.package`.** The first revision's scenario
text claimed this, while its predicate accepted it - the contradiction round 2
caught. Refusal was reconsidered on its merits and rejected: `lib.toDerivation`
yields one determinate output, the layout check, the digests, and the signature
chain all run against that path unchanged, and `lib.types.package` has admitted
store paths for as long as the option type has existed. Refusing would break a
legitimate authoring form to no security end. The predicate was already right;
the prose was wrong.

**Detecting non-ELF entries at launch instead of at build.** Cheaper: the kernel
reports `ENOEXEC` and the launcher maps it. Rejected because the failure would
then depend on the sandbox profile - a `#!` entry point works where `/proc` is
mounted and fails where it is not - so the same artifact would be valid on one
host and not another, discovered only in production. Item 3 pays a bounded
18-octet read at build time to make the answer the same everywhere.

**Modelling `binary_ref` as `Option<BinaryRef>`.** The obvious shape, and it is
what a serde-derived DTO would produce by default. Rejected because `None` would
carry two meanings - "not launchable" and "launchable, field omitted" - and the
second is the exact hole item 3 exists to close. `ComponentExecution` makes the
launchable arm carry the ref, so the two facts cannot disagree, and confines the
absent case to a closed bootstrap list.

**Serializing `ComponentExecution` with a serde enum representation.** Adjacent
or internally tagged (`{"execution":{"type":"launchable","binaryRef":"..."}}`)
would make the JSON self-describing and would let a reader see the variant
directly. Rejected because it changes the manifest bytes that section 4.3.3
specifies and every Provider dossier already writes, which means a
`schemaVersion` bump, a `manifestDigest` change on every existing artifact, and a
migration for a problem that does not exist: the flat field is unambiguous once
the admission rule decides whether an omission is legitimate. Item 14 keeps the
wire flat and resolves the variant at the parse boundary, so no version moves.

**Deriving `Deserialize` on `BinaryRef` with `#[serde(transparent)]`.** One line
instead of six. Rejected because it would accept any string from a signed
manifest, which is the only input path that matters, leaving the item 3 grammar
enforced solely for values constructed in Rust. `ArtifactId` in the same module
already hand-writes the validating impl for exactly this reason.

**Leaving interpreted Providers to solve the ELF rule themselves.** The round-2
position, and the honest reading of it is that it exported a cost the framework
created and invited a workaround. Each publisher would write the same twenty-line
C shim, most would reach for `makeWrapper` first and discover it emits a shell
script, and some would conclude the rule was arbitrary. Item 3a costs the
framework one small helper and converts a prohibition into a supported path.

**Calling `openat2`, `fstat`, and `execveat` directly at each call site.** Less
code and no trait objects. Rejected because the sequencing *is* the security
property, and a sequence spread across call sites can only be tested with a
privileged fixture that builds real symlinks and really execs. The boundary in
item 8a makes the ordering, the by-value single-use discipline, and the errno
mapping hermetically testable, and leaves exactly one module naming the
syscalls.

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
