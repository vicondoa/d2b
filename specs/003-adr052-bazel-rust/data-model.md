# Data Model: ADR 0052 Bazel Rust Gate

These are internal migration and evidence entities, not application data or a
new public API. The existing execution-manifest v1 reference and schema remain
authoritative.

## Modelling rules

Two rules apply to every entity below, because the round-one plan panel found
both classes of defect in an earlier draft:

1. **Variants, not a tag plus optional members.** Where a record has several
   shapes, the shape is the variant and the variant carries its own members. A
   `kind` field beside members that only some kinds may use admits states that
   cannot exist, and a validator then has to re-derive the invariant that the
   type should have made unrepresentable.
2. **No constant fields.** A field whose only legal value is a constant is not
   data; it is an invariant, and it belongs in prose where it cannot be set to
   the other value. Global invariants are listed once, below.

Global invariants, formerly modelled as always-true or always-false fields:

- No carrier action opens a network socket.
- Every carrier reports an independent verdict.
- No fixture-backed identifier is part of this model; the two fixture surfaces
  stay on the Cargo and Nix path.
- The two locator arms never chain, no located path is an absolute
  execution-root path, and a located provider is opened once and executed
  through that same descriptor.
- Concurrency is always derived from `D2B_RUST_BUDGET`, which remains the only
  resource control.
- Under Bazel, every binary is resolved through declared runfiles, and only the
  declared test environment is forwarded to a child.
- Repository-owned execution paths are shell-free. The
  `rules_rust`-generated stable-channel doctest runner is not repository-owned
  and is a recorded deliberate difference.
- Result publication is enforcing for every test carrier.

## Rust Surface

Common to every variant:

| Field | Rule |
| --- | --- |
| `surface_id` | Unique member of the fixed eighteen-ID baseline. |
| `cargo_baseline` | Current Make leaf/mode and command family. |
| `carriers` | Nonempty set of Carrier Targets; exactly one owns the verdict. |
| `slice_id` | Exactly one of `main`, `api`, `broker`, `aux`. |

The variant is the shape, and each variant carries only what it can have:

| Variant | Members | Baseline identifiers |
| --- | --- | --- |
| `Compile` | none beyond the common set | `rust-main-format`, `rust-main-clippy` |
| `TestSuite` | every carrier is a Test Carrier and therefore has a topology | `rust-main-workspace-tests`, `rust-guest-shell-runner`, `rust-broker-default`, `rust-broker-layer1`, `rust-broker-fakebackends` |
| `Policy` | `check_inputs`: the committed policy files, pinned snapshots, and pinned artifacts the carriers declare, nonempty | `rust-deny-main`, `rust-deny-broker`, `rust-deny-guest`, `rust-audit-main`, `rust-audit-broker`, `rust-audit-guest`, `rust-stub-no-socket` |
| `Census` | `census_ref`: one generator-derived census artifact plus its derivation | `rust-api-surface`, `rust-assert-pinned` |
| `Scan` | `governed_source_ref`: the exact generated input manifest | `rust-no-bash-ast` |
| `Reproducibility` | `emitted_census_ref`: the census the generator returns, not a literal | `rust-schema-reproducibility` |

A `Compile` surface has no census member to leave empty and a `Policy` surface
has no topology member to leave absent, so neither state is expressible. The
identifier column is the current assignment and is itself checked against the
coverage map; moving an identifier between variants is a contract decision, not
a map edit.

The mapping is total and unambiguous: every `surface_id` has a nonempty
`carriers` set, and every carrier belongs to exactly one `surface_id`.
Cardinality one is not required and never was; `rust-main-workspace-tests`
already needs three carriers. Removing or adding a baseline ID requires a
separate contract decision, not a map edit.

## Carrier Target

Common to both variants:

| Field | Rule |
| --- | --- |
| `label` | Unique Bazel label; ADR-fixed labels live below `//ci/rust`. |
| `surface_id` | Exactly one Rust Surface. |
| `owns_verdict` | True for exactly one carrier per surface. |
| `declared_inputs` | Closed, nonempty input set. |
| `declared_outputs` | Exact outputs, if any; generated outputs must be nonempty. |
| `handwritten_fragments` | Every non-generated BUILD fragment used. |
| `runfiles_data` | Every binary and fixture this carrier's actions locate, as declared data. |
| `binary_identities` | The expected byte digest of every executable in `runfiles_data`, one per executable, no more and no fewer. Each is compared against the descriptor the provider was opened on, never against a second open. |

Variants:

| Variant | Members |
| --- | --- |
| `TestCarrier` | `topology`: exactly one Test Topology. `test_targets`: the Rust test targets carried, nonempty. `result_document`: the Per-Case Result Document this carrier publishes. |
| `CheckCarrier` | `check_inputs`: the committed configuration, snapshot, manifest, or pinned artifact the check consumes, nonempty. |

Topology and the per-case result document belong to the carrier, not to the
surface. `rust-main-workspace-tests` carries a process-per-case suite, a
doctest carrier, and a harness-free carrier, and those are three different
topologies under one identifier; a surface-level topology field could not
represent that without lying about two of them.

Label existence is proved at analysis time by a real dependency edge from the
coverage guard, not by a query issued from inside a test. Every Rust test
target and hand-written fragment is claimed exactly once across carriers.

## Coverage Map

| Field | Rule |
| --- | --- |
| `baseline_source` | Existing execution-manifest reference. |
| `surfaces` | Exactly eighteen Rust Surface records, sorted by ID. |
| `carriers` | Referenced Carrier Targets, with no orphan and no carrier claimed twice. |
| `slices` | Fixed four-slice assignment. |
| `generated_build_digest` | Digest of generated first-party BUILD tree. |
| `governed_source_manifest` | Exact no-bash input manifest. |
| `derived_censuses` | Generator-derived executed harness-free, doctest, and emitted-schema censuses. |
| `out_of_census_entries` | Every manifest entry the executed selector excludes, each with its reason. |
| `handwritten_fragments` | Every non-generated fragment, including the channel transition rule, the `rustdoc_json` rule, the vendor repository rule, and the yanked-state carrier fragment. |
| `query_result_ref` | The committed drift-checked graph query result the out-of-test completeness check consumes. |
| `locator_migration` | Reference to the Test Locator Migration record set. |
| `deliberate_differences` | ADR section 13 difference and rationale per affected surface. |

Validation is bidirectional: every baseline ID has one map row, every mapped
label exists at analysis time, every test target and fragment is claimed
exactly once, and every referenced census and topology exists. A minimum count
is invalid where an exact derivation exists, and a literal count committed by
hand is invalid where the generator can derive one.

## Test Topology

Common to every variant:

| Field | Rule |
| --- | --- |
| `topology_id` | Unique stable internal ID. |
| `carrier_label` | The one Test Carrier this topology describes. |
| `case_tmpdir` | Each unit of execution gets its own directory beneath the executor temporary root. |

Variants:

| Variant | Members |
| --- | --- |
| `ProcessPerCase` | `suite`: main workspace or guest shell runner. `case_census`: exact nonempty libtest listing. `ignored_census`: exact ignored names and count. One fresh process per case. |
| `ProcessPerBinary` | `suite`: one broker feature suite. `binary_census`: exact nonempty binary listing. `case_census` and `ignored_census` as above. `internal_threads`: positive bounded value. Exclusive by construction. |
| `Doctest` | `discovered_census`: derived, nonempty. |
| `HarnessFree` | `discovered_census`: derived, nonempty, matching the selector the Cargo gate uses. |

`internal_threads` exists only where a binary runs several cases in one
process, and exclusivity is a property of the `ProcessPerBinary` variant rather
than a boolean any topology could set. Exclusive carriers run one at a time and
strictly after the parallel phase, which is a property of the schedule rather
than of the carrier. `Doctest` and `HarnessFree` discovery is derived and
refuses an empty result; those two variants carry a census rather than a
process contract, so the qualification evidence records exactly five topology
proofs, two `ProcessPerCase` and three `ProcessPerBinary`.

## Per-Case Result Document

| Field | Rule |
| --- | --- |
| `path_source` | The path the executor supplies through `XML_OUTPUT_FILE`. |
| `entries` | One per enumerated case, with `passed`, `failed`, or `ignored`. |
| `permitted_content` | Stable case name, outcome, bounded duration, bounded sanitized failure text. |
| `forbidden_content` | Environment values, arguments, absolute paths, store paths, socket paths, the runfiles root and any resolved absolute runfiles or worktree location, unit names, PIDs, UIDs, opaque handles, terminal bytes, shell names, raw child output. |
| `raw_output_location` | The ordinary per-target `test.log` artifact only. |
| `write_semantics` | Anchored close-on-exec parent descriptor, link and magic-link refusal, close-on-exec same-directory temporary, sync, descriptor-relative rename. |
| `ownership` | Only a runner-created temporary is ever unlinked; a failed creation unlinks nothing. |
| `ordering` | No output descriptor is opened before every child is reaped. |
| `failure_precedence` | An existing test failure remains primary; publication failure is reported additionally. |

Publication is enforcing, per the global invariants: a passing carrier fails
when publication fails. Every property in this table has a planted mutation the
test must reject, and every one of those mutations is produced through the
injected boundaries below rather than by arranging host state.

## Injected Boundaries

Four boundaries exist so that failure states are supplied rather than
provoked. All are W0-frozen module paths, so later scopes open against a stable
surface. The first two live in `packages/d2b-bazel-support/`, the neutral
internal crate that declares no first-party dependency, because the runner, the
locator, `xtask`, and, as a dev-dependency only, `packages/d2b-contract-tests`
all read them; the crate exists so that no consumer has to depend on another
consumer to reach a boundary.

| Boundary | Path | Serves | Supplied states |
| --- | --- | --- | --- |
| `FileSystem` | `packages/d2b-bazel-support/src/fsops.rs` | Per-case result publication, scratch cleanup, wave-note corpus enumeration, and every provider open, check, and execution | `openat2` and forced component-walk routes, both resolve policies on each route, a leaf symlink and an intermediate symlink under each policy, magic-link parents, anchored `..` escape, `EEXIST` collision, short write, `EINTR`, `EAGAIN`, `ENOSPC`, replacement race, tracked entry, foreign decoy, an unreadable and an empty note corpus, note-directory enumeration returned in two different orders, note entries failing `EACCES`, `EISDIR`, `ELOOP`, and non-UTF-8 content, a note entry whose raw name is not valid UTF-8 and a second whose raw name differs from it only in bytes a lossy conversion would collapse, an absent, non-regular, non-executable, out-of-date, or wrong-digest provider, a path rebound to a different inode after the provider open, handle metadata that changes across the digest read, and `spawn_verified` returning `ENOSYS`, `EACCES`, `ENOEXEC`, `ENOENT`, or `ETXTBSY` |
| `RunfilesView` | `packages/d2b-bazel-support/src/runfiles.rs` | The locator's Bazel arm and the runner's child-binary resolution | A declared entry present, a declared entry missing, and a runfiles environment that indicates no Bazel test at all |
| `Clock` and `UptimeSource` | `packages/d2b-bazel-runner/src/clock.rs` | Deadline parsing, remaining-budget arithmetic, child duration, expiry escalation | Every accepted and rejected uptime field, truncate on capture and round up on read, exactly-zero remaining budget, overflow, expiry reached without sleeping |
| `YankedIndex` | `packages/xtask/src/bazel_yanked.rs` | The reviewed networked yanked-snapshot refresh | All-clear index, a yanked version, a key the locks declare and the index omits, a key no lock declares, a missing index revision, a transport failure, a malformed payload |

Execution of a verified provider is an operation on `FileSystem` rather than a
fifth boundary. Splitting verification and execution across two injectable
traits would let a composition satisfy both fakes while still executing by
path, which is the exact defect the single-open rule removes; keeping them on
one trait makes "holds a verified handle" and "can reach an execution route"
the same reachability question.

The resolve policy is one parameter with two routes and one meaning. On the
`openat2` route it selects the resolve flags; on the forced component-walk
route it selects `O_NOFOLLOW` on the final component, which every intermediate
component carries under both policies. Strict callers are cleanup, the per-case
directories, the `XML_OUTPUT_FILE` parent, and the wave-note lint; the provider
open and each declared input its freshness check reads are the permissive ones.
The route never changes what a policy means, which is why the fake supplies
both routes for both policies and each call site's choice is asserted.

`Clock` and `UptimeSource` stay in the runner rather than moving to the support
crate, because only the runner's deadline and process paths read them. The
locator needs no clock: provider freshness compares two timestamps the
`FileSystem` boundary already returns from the provider's own descriptor, and
provider identity is a byte digest read from that same descriptor rather than
an execution.

## Verified Executable Handle

The record that makes provider verification and provider execution one
operation rather than two resolutions of one name.

| Field | Rule |
| --- | --- |
| `anchor` | Close-on-exec directory descriptor: the runfiles root under Bazel, the parent of the `CARGO_BIN_EXE_<name>` value under Cargo. |
| `relative` | One declared relative path, never absolute, never empty, never carrying `..`. |
| `descriptor` | Exactly one `O_RDONLY` plus `O_CLOEXEC` open of `relative` beneath `anchor`, resolved with `RESOLVE_NO_MAGICLINKS`, or on the forced component-walk route with `O_NOFOLLOW` on every component except the leaf. `O_PATH` is invalid here because identity requires reading the bytes. |
| `stat_before` and `stat_after` | `fstat` on the descriptor immediately before and immediately after the digest read; `st_dev`, `st_ino`, `st_size`, `st_mtim`, and `st_ctim` must agree. |
| `kind_and_mode` | Regular file with an executable mode, from `stat_before`. The kernel's `EACCES` at exec time maps to the same refusal. |
| `freshness` | `stat_before.st_mtim` at least the newest declared input's, each input read from its own descriptor through the same boundary. |
| `identity` | Digest of `stat_before.st_size` bytes read from the descriptor at offset zero, equal to the value the coverage map records for this provider. |
| `execution` | `execveat(descriptor, "", argv, envp, AT_EMPTY_PATH)` in a forked child. No `Command`, no `fexecve`, no `/proc/self/fd/<n>`, and no fallback on `ENOSYS`. |
| `ownership` | The parent holds the descriptor for the whole carrier invocation and closes it through the boundary after the last child using it is reaped, still before any output descriptor opens. |
| `child_inheritance` | The three stdio descriptors only. Close-on-exec removes the provider descriptor from the child; a non-close-on-exec control descriptor proves the assertion can fail. |

There is no `path` field and no accessor that yields one, because a path is
what a second resolution would need. The type has no public constructor: it is
produced only by consuming a provider handle that passed every check above, so
"unverified executable" is not a representable state and a compile-level test
asserts it. One handle serves every case of a process-per-case topology, which
is what makes "every case ran the bytes that were digested" true rather than
merely likely.

Every row above holds identically on both resolution routes. A leaf the
provider policy accepted through a symlink is still `fstat`ed for kind and
mode, still compared for freshness, still digested from offset zero to
`st_size` against the coverage map's value, and still `fstat`ed again after the
read. The route decides only what may be traversed to reach the leaf; it never
decides what is proved about the descriptor that comes back.

## Wave-Note Lint Refusal

The type-5 policy lint's outputs, modelled because an earlier draft rendered
one remedy for two unrelated conditions.

Entry, as the enumerator returns it:

| Field | Type | Why this type |
| --- | --- | --- |
| `name` | `std::ffi::OsString` | The raw directory-entry name, exactly the bytes the enumeration returned. A Linux directory entry is any NUL-free, `/`-free byte string; it is not a `str`. |
| `content` | `std::io::Result<String>` | Exactly what the boundary read returned, never mapped onto `None` and never onto an empty string. |

**The name is raw bytes because `String` cannot hold every name the kernel
permits.** An enumerator whose name field is `String` has only two ways to
handle an entry the directory really contains and UTF-8 cannot represent, and
both break a fail-closed guard. Dropping the entry is the worse one: the
`w<digits>.md` name-shape refusal is what makes *anything else in this
directory* a refusal, so an entry the enumerator never returns is an entry that
rule never sees, and a guard that silently omits the one entry a contributor
could not name is fail-open in the position it is fail-closed everywhere else.
Lossy conversion is the other, and it is not benign either. Measured on this
host with the pinned stable toolchain: the distinct raw names `w\xff9.md` and
`w\xfe9.md` both convert to the identical lossy text, so two entries collapse
onto one rendered label and onto one sort key, and the tie is then broken by
directory order, which is the exact irreproducibility the sort exists to
remove. Lossy conversion also inverts the order against clean names, not only
against other broken ones: raw `w\x80.md` sorts before the perfectly valid
UTF-8 name `w\xc3\xa9.md`, while their lossy forms sort the other way, because
`U+FFFD` encodes to `0xEF 0xBF 0xBD` and outranks `0xC3`. A `Position` label
derived from a lossy sort therefore names the wrong entry for entries that were
never broken.

Holding `OsString` costs nothing at the boundary. Measured: the enumeration
returns those names unchanged, `CString::new(name.as_bytes())` round-trips each
one for the descriptor-relative open, and a mode `0000` or symlinked entry with
a non-UTF-8 name still yields its own `EACCES` or `ELOOP`. The type change
moves where UTF-8 is required, from the corpus to the renderer, which is the
only place it was ever needed.

Corpus level, returned instead of an entry list:

| Variant | Members | Remedy |
| --- | --- | --- |
| `Unreadable` | The real `std::io::Error` from the anchored open or the entry enumeration | Restore `specs/003-adr052-bazel-rust/wave-notes/` and its permissions. |
| `Empty` | none | Add this wave's note under `specs/003-adr052-bazel-rust/wave-notes/`. |

Entry level:

| Variant | Members | Remedy |
| --- | --- | --- |
| `PathLeak` | `NoteLabel` and the one-based line | Rewrite the path as a `<worktree>`-rooted shape or drop it. |
| `ReadError` | `NoteLabel` and the preserved `std::io::Error` | Fix the entry's permissions or remove the invalid entry. |

`NoteLabel` is `Name(String)` or `Position(NonZeroUsize)`. The `String` in
`Name` is a rendered label, never the stored name: it exists only where
`OsStr::to_str()` on the raw name returned `Some`, so the conversion is a check
that can fail rather than one that launders.

| Variant | When |
| --- | --- |
| `Name` | `OsStr::to_str()` on the raw name returns `Some`, and that `&str` passes the lint's own `/`-rooted-token and worktree-substring rules. |
| `Position` | Anything else: a name that carries a leak, or a name whose `to_str()` is `None` and which therefore has no rendering. The value is the entry's one-based index in the **sorted** enumeration. |

`PathLeak` has no error member to be absent and `ReadError` has no line member
to be zero, so neither impossible state is expressible. No variant carries the
offending token and none renders an absolute path. The note label is checked
before it is rendered, which is what makes the self-application test a property
rather than a coincidence about the names that happen to be committed.
Remedies cannot be borrowed across variants, and a test asserts per variant
that the other three remedies are absent; the assertion is on the whole remedy
sentence, because the two corpus remedies deliberately share the directory
literal.

The two corpus remedies name the corpus, and they name it as the fixed
repository-relative literal `specs/003-adr052-bazel-rust/wave-notes/`. A corpus
error names no entry, so a remedy without the directory tells the contributor
to repair something unnamed. The literal is compile-time text, never the
rendered form of the path the enumerator opened beneath `repo_root()`: that
rendered form is an absolute path, FR-029 forbids one in a refusal, and the
lint's own self-application case would catch its own message. The literal is
safe under the lint's rules by construction, because every `/` in it is
preceded by an ordinary path character and so is not a `/`-rooted token.

**Enumeration order is defined, not inherited.** The enumerator sorts entry
names by unsigned byte order over `OsStr::as_bytes()` before opening anything,
and the returned entry sequence, the violation order, and every `Position`
value derive from that sorted sequence. The comparison names `as_bytes()`
rather than leaning on `OsString`'s own `Ord`, whose relation to the raw bytes
the standard library does not promise across targets; measured here the two
agree and `0x80` sorts above `0x7f`, and pinning the byte comparison is what
keeps that agreement from being load-bearing. Measured, the same seven note
names enumerate as
`w2 w0 w1 w11 w3 w10 w9` on ext4 and `w3 w11 w1 w0 w2 w10 w9` on tmpfs, so an
unsorted `Position` names a different entry in CI than it does locally and the
refusal is not reproducible from the message. Byte order rather than a locale
collation, because it is total over raw directory-entry bytes and identical
everywhere. Total is the operative word: the corpus may hold a name no locale
collation is defined over, which is the second reason the sort key is the raw
bytes and not a rendered string.

`ReadError` preserves the boundary's error unchanged for `EACCES`, `EISDIR`,
`ELOOP`, and non-UTF-8 content. The one constructed error is
`ErrorKind::InvalidInput` for an entry whose name does not match `w<digits>.md`
and which is therefore never opened; a test pins the exact `raw_os_error` of
the other four so this stays the only construction.

A non-UTF-8 *name* and non-UTF-8 *content* are different conditions and must
not be conflated. Content is read and its decoding failure is
`ErrorKind::InvalidData` on an entry the lint did open. A name is never
decoded: the shape rule runs over the raw bytes, so `w\xff9.md` fails
`^w[0-9]+\.md$` because `0xff` is not a digit, and the entry is refused
`ErrorKind::InvalidInput` without being opened, carrying a `Position` label
because it has no rendering. One planted entry, one refusal, no silent
omission. This is the state the previous `String` name field could not reach.

No test of cleanup, result publication, deadline handling, wave-note corpus
enumeration, or provider resolution may depend on live host filesystem state, a
full disk, a privileged mount, or the host clock. A property that can only be
exercised by arranging host state is a property that will be marked ignored,
which is the same as not testing it. That applies with particular force to the
stale-provider case: a test that writes an out-of-date executable into
`packages/target/` has planted the exact hazard the locator exists to refuse,
on the host every other suite shares, and leaves it there if the run is
interrupted.

One test is exempt, deliberately and by name. Every claim the Verified
Executable Handle makes about `execveat`, close-on-exec inheritance, and
repeated execution of one descriptor is a claim about the kernel, and a fake
cannot prove a kernel. `packages/d2b-bazel-runner/tests/exec_handle.rs` drives
the host-backed implementation against the first-party probe binary
`packages/d2b-bazel-runner/src/bin/d2b-exec-probe.rs`, which reports its own
descriptor table. It arranges nothing on the host and writes no executable
anywhere; it executes a declared input the graph already builds, which is what
the runner does in normal operation.

The same rule holds for the network. `IndexClient` is the single networked
implementation of `YankedIndex` and the only site permitted to open a socket
for the refresh; every unit test of the refresh injects a fake instead, so no
test resolves a name or reaches the live index. `bazel-yanked-check` names
neither the trait nor its networked implementation, which is what makes the
offline validator offline by construction. Real-index behavior is measured
separately, by the reviewed contributor-run refresh whose diff and observed
index revision the committing wave records.

## Test Locator Migration

Every record identifies one affected first-party file and one of two
dispositions. The disposition is the variant, so a record cannot claim to need
no migration while also carrying a declared runfiles-relative path.

Common:

| Field | Rule |
| --- | --- |
| `file` | Repository-relative path of one affected first-party file. |
| `site` | `binary-location`, `manifest-path`, or `repo-root-walk` with the helper named. |

Variants:

| Variant | Members |
| --- | --- |
| `Migrated` | `bazel_runfiles_path` and the `data` label providing it; `cargo_call_site_crate`, the test crate the Cargo arm expands in; for a `binary-location` site, the Verified Executable Handle record, whose digest the located descriptor must match before that same descriptor is executed. |
| `NoMigrationNeeded` | `reason`: the recorded reason this file needs no change. |

The affected set is enumerated, not sampled: 25 files locating binaries through
compile-time Cargo environment expansion and 20 test files resolving
`CARGO_MANIFEST_DIR`, 11 of those through a `repo_root()` helper. A file that
is in neither variant is a gap the coverage map makes visible. Both arms stay
green on the Cargo path for the whole shadow stage.

Provider negatives are supplied, never arranged. The absent, non-regular,
non-executable, out-of-date, and wrong-identity providers, the path rebound to
a different inode after the open, and the missing runfiles entry that turns a
Bazel-mode lookup into a refusal, are all states of the `FileSystem` and
`RunfilesView` fakes in `packages/d2b-bazel-support/`. No record in this set is
proven by writing an executable to `packages/target/` or to any other live
path, no provider check executes a provider to learn its identity, and no
migrated call site spawns by path: identity is the digest of the descriptor's
bytes, and execution is `execveat` on that same descriptor.

## Hermeticity Inventory

| Field | Rule |
| --- | --- |
| `hub` | One of the four `crate_universe` hubs: `main`, `broker`, `guest`, `walker`. |
| `hub_lock_attrs` | `lockfile`, `cargo_lockfile`, and `skip_cargo_lockfile_overwrite = True` are all present. |
| `build_script_crates` | Every third-party crate for which a build-script target is generated. |
| `required_annotations` | Per crate: build-script environment, data, and toolchain requirements. |
| `action_env_allowlist` | The explicit minimal set of host environment values any action may observe. |
| `bazelignore_entries` | `.scratch/` plus every Cargo output directory any workspace or tool creates. |
| `symlink_prefix` | Absolute path beneath `.scratch/`. |
| `startup_options` | Absolute values supplied by the wrapper from the one construction in `packages/d2b-bazel-support/src/startup.rs`, byte-identical across build, test, query, info, shutdown, clean, and, from W2, the repin and module-refresh children. |
| `generator_pin` | `cargo-bazel` URL plus sha256; source bootstrap refused. |
| `module_lock_modes` | `.bazelrc` carries `common --lockfile_mode=error` and `common --check_direct_dependencies=error`; neither may be relaxed by a wrapper argument. |

Repin controls are absent from the wrapper and from every
continuous-integration environment. The single scoped exception is the child
environment `cargo xtask bazel-repin --hub <name>` constructs, which sets
`CARGO_BAZEL_REPIN` and `CARGO_BAZEL_REPIN_ONLY=<hub>` for that one process,
writes only that hub's Bazel-side lock, and fails when any other tracked
derived artifact changed. `cargo xtask bazel-module-refresh` sets no repin
control at all, refuses to run when one is ambient, writes only
`MODULE.bazel.lock`, fails when any other tracked derived artifact changed, and
changes nothing on an already-current tree. Neither command is a Make target or
reachable from a workflow.

Every field here is a cache-key input. A change to `action_env_allowlist`
invalidates the entire action cache and is reviewed against the promoted size
budget in the same change.

## Execution Manifest Binding

| Field | Rule |
| --- | --- |
| `authority` | `docs/reference/test-execution-manifest.md` and its v1 schema. |
| `executor` | `cargo` during shadow; `bazel` after promotion. Not a schema field. |
| `surface_mapping` | Carrier result to existing surface ID. |
| `completed_mapping` | Success only after all commands for that surface complete. |
| `failure_mapping` | Observed carrier failures to existing `failed_surfaces`. |
| `interruption_mapping` | Handled interruption publishes available partial evidence. |

This binding cannot add, rename, reinterpret, or version manifest fields.
Prior evidence is invalidated before dispatch. Passing promotion evidence
requires a v1 `passed` manifest with all eighteen IDs; partial evidence is
diagnostic only.

## Qualification Record

| Field | Rule |
| --- | --- |
| `head_sha` | The commit both workflows tested; identical for both run IDs. |
| `source_event` | `push` on `refs/heads/v3` produced by a merged pull request. |
| `bazel_run_id` | Unique immutable shadow workflow run ID. |
| `cargo_run_id` | Unique immutable required workflow run ID at the same `head_sha`. |
| `cargo_verdict` | `passed` or `failed` from `D2B_SKIP_FIXTURE_BUILD=1 make test-rust`. |
| `bazel_verdict` | `passed` or `failed` from the Bazel rollup. |
| `fixture_verdict` | `passed` required, from same-commit `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`. |
| `slice_verdicts` | Exactly four attributed results. |
| `slice_seconds` | Four complete job durations; required for a cold-sample record. |
| `manifest_ref` | Immutable evidence reference. |
| `cache_restored` | Must equal zero for a qualifying cold sample. |
| `cache_writes` | Must equal zero during the shadow stage. |
| `permissions` | PR-reachable jobs: only `contents: read`; no `actions: write`. |

Records are ordered by `v3` push completion. The promotion streak is ten
consecutive records whose two compared verdicts match with a passing
`fixture_verdict`. Streak arithmetic is fail-closed:

- differing verdicts reset the streak to zero;
- a Bazel run that reaches no verdict while its paired Cargo run reaches one
  counts as a mismatch and resets the streak;
- a push where neither side reaches a verdict is not a record and neither
  extends nor resets.

Pull-request, `main`-push, scheduled, and dispatched runs are diagnostic. They
never enter a streak or a measurement set, because `refs/pull/N/merge` is
recomputed against a moving base and a Bazel-path-filtered pull-request sample
cannot contain the divergence class the streak exists to detect.

## Seeded Failure Record

| Field | Rule |
| --- | --- |
| `surface_id` | Unique across the evidence set; all eighteen required. |
| `seed_commit` | Immutable disposable commit or patch digest. |
| `seed_description` | The single protected invariant intentionally broken. |
| `invoked_make_target` | Owning approved slice or aggregate target. |
| `expected_carrier` | Carrier for `surface_id`. |
| `observed_failed_surfaces` | Exactly `[surface_id]`. |
| `unrelated_failures` | Empty. |
| `partial_manifest_ref` | Failed v1 manifest reference. |

A record is invalid if the seed changes more than one protected condition or
if an unrelated surface fails.

## Performance Measurement Set

| Field | Rule |
| --- | --- |
| `profile` | `warm-local`, `cold-local`, or `cold-ci`. |
| `environment` | ADR reference local host or runner facts and tool pins. |
| `sample_commits` | One SHA per sample. Local samples use one candidate SHA; cold-CI samples use each record's `head_sha`. |
| `sample_refs` | Run IDs for every sample; cold-CI refs also carry the `push`-on-`v3` source event. |
| `cache_state` | Exact ADR profile; warm records the edit and live server, cold local retains only the repository cache, cold CI restores nothing. |
| `invocation_flags` | The exact flags each sample ran under; `--test_output=streamed` invalidates the sample. |
| `samples_seconds` | Three local samples, or the five most recent qualifying cold qualification records. |
| `qualifying_rule` | A cold-CI sample qualifies only when no Bazel cache was restored and all four slice jobs completed with a recorded duration. |
| `ceiling_seconds` | 600 warm; 900 cold local and cold CI. |
| `feasibility_ref` | Required for `cold-ci`: the W3 feasibility measurement that made the ceiling binding, or the pre-authorized remedy taken instead. |
| `median_seconds` | Computed over all required valid samples. |
| `maximum_seconds` | Maximum sample. |
| `output_root_sizes` | Before/after for local samples. |
| `valid` | True only if median is at/below ceiling and max is at/below 1.2 times ceiling. |

A cleanup, hard refusal, server restart, wrong edit, cache-state change, heavy
lane overlap, streamed test output, or mismatched environment invalidates a
sample. Invalid samples are retained with their reason and replaced; they do
not enter the median. The `api` slice's samples include the second
configuration the channel transition creates; that cost is inside the ceiling,
not carved out of it.

## Cache Generation

| Field | Rule |
| --- | --- |
| `generation_id` | Unique successful protected-`v3` run identifier. |
| `kind` | `action` or `repository`; never `output-base`. |
| `key_input_digest` | Digest over `.bazelversion`, `MODULE.bazel`, `MODULE.bazel.lock`, `.bazelrc`, both `rust-toolchain.toml` files, all four hub Cargo locks, `packages/Cargo.guest.lock`, all four per-hub `crate_universe` Bazel-side locks, the `cargo-bazel` URL and sha256, all deny configurations, the advisory-database pin, the committed yanked snapshot, `.bazelignore`, the symlink-prefix and startup-option configuration, the build-script annotation and action-environment digest, and the generated BUILD tree digest. |
| `restore_prefix` | Omits run ID and commit SHA. |
| `trim_evidence` | Reference proving the explicit synchronous collector completed before measurement. |
| `size_bytes` | At most 4 GiB action or 1 GiB repository, measured after the trim. |
| `writer_job` | Same single protected-`v3` writer for both coordinated saves. |
| `source_event` | Protected-`v3` push only. |
| `state` | `planned`, `restored-read-only`, `trimmed`, `published`, `superseded`, `deleted`. |

PR jobs can only reach `restored-read-only`. Publication requires complete
maintenance pagination, unambiguous authorized prefixes, an observed
synchronous trim, and two checks that repository usage plus planned snapshot is
at most 8 GiB. Credentials cannot enter a run step or a Bazel environment. Any
key input changing without changing the key is a defect, not a tuning choice.

## Recovery Condition

Common:

| Field | Rule |
| --- | --- |
| `code` | Unique stable static code. |
| `trigger` | One exact refusal or expiry class. |
| `message_template` | Fixed and actionable. |
| `required_steps` | Exact repository-relative remedy for this code. |
| `forbidden_values` | Absolute path, output hash, user/PID, raw deadline, opaque handle. |
| `forbidden_actions` | Code-specific unsafe actions. |

Variants:

| Variant | Members | Codes |
| --- | --- | --- |
| `CleanupRefusal` | Deletes nothing, by construction of the variant. Exercised through the injected `FileSystem`. | `D2B-BZLCLEAN-TRACKED`, `D2B-BZLCLEAN-SYMLINK`, `D2B-BZLCLEAN-ESCAPE`, `D2B-BZLCLEAN-LIVE` |
| `ServerRefusal` | Bounded shutdown attempt, no manual signal instruction. | `D2B-BZLSERVER-STUCK` |
| `DeadlineOutcome` | `measured_duration` and `target`. Exercised through the injected `Clock`. | Expired budget and ceiling miss |

`deletes_nothing` is not a field, because only `CleanupRefusal` can carry it
and it is always true there. Expired budget and ceiling miss are normal
deadline outcomes rather than refusals. Remedies cannot be
borrowed across codes. A ceiling miss names only a larger runner or further
disjoint split.

## Qualification Evidence Record

This is the concrete immutable record for the feature specification's
Promotion Evidence Set before executor authority changes.

| Field | Validity rule |
| --- | --- |
| `candidate_commit` | One immutable integrated commit. |
| `coverage_map_digest` | Both guard halves pass for all eighteen. |
| `qualification_records` | Ten consecutive matching push-to-`v3` records, each with one shared `head_sha`, both run IDs, and a passing fixture-contract verdict. |
| `seeded_failures` | Exact eighteen-record set. |
| `topology_proofs` | Main, guest, and three broker suites; exact generator-derived censuses and ignored counts, plus per-case result publication. |
| `locator_migration_proof` | Every enumerated file migrated or recorded as needing none, plus the passing injected stale-provider negative in which the `FileSystem` fake reports an out-of-date, wrong-digest executable at the Cargo path while the `RunfilesView` fake reports the entry missing, plus the passing injected post-open path-rebind negative, plus the host-backed `execveat` conformance result. |
| `broker_repetitions` | Twenty consecutive passes per broker suite with exclusivity. |
| `performance_sets` | Three valid profiles. Local sets bind the candidate; cold-CI samples carry their own `head_sha` values and reference the W3 feasibility measurement. |
| `supply_chain_comparison` | Three locks, no differing enforcing outcome, with the yanked carrier landed and `cargo xtask bazel-yanked-check` passing offline against all three. |
| `cache_shadow_proof` | Zero shadow publications. |
| `workflow_policy_proof` | Positive and every required negative fixture pass. |
| `status` | `collecting`, `qualified`, or `invalidated`. |

Before W4 merge, any candidate-content change invalidates evidence tied to
affected content and returns the draft to `collecting`. `qualified` is
required before promotion. Once committed as `qualified`, the record is
immutable. Promotion references its digest and does not mutate it.

Historical qualification records are a sequence, not candidate-owned samples.
Each retains its own `head_sha` and run IDs. Candidate-bound coverage,
seeded-failure, topology, locator, local-performance, and supply-chain evidence
must match `candidate_commit`.

## Promotion Record

| Field | Rule |
| --- | --- |
| `promotion_commit` | Immutable SHA that changes executor authority. |
| `qualification_digest` | Digest of the immutable qualified W4 record. |
| `maintenance_run_id` | Protected-`v3` cache maintenance run. |
| `deleted_generations` | Only authorized retired/superseded keys. |
| `trim_evidence` | Reference proving the synchronous collector completed before both headroom checks. |
| `headroom_checks` | Both pre-save checks at or below 8 GiB. |
| `writer_run_id` | The one authorized publishing job. |
| `first_promoted_verdict` | Required `test-rust` verdict after promotion. |
| `rollback_rehearsal` | Reference proving one revert restores Cargo authority. |

The record is written after the ordered protected-`v3` promotion run and is
immutable once reviewed.

## Post-Promotion Observation

| Field | Rule |
| --- | --- |
| `promotion_commit` | Must equal Promotion Record SHA. |
| `release_tags` | Tags that contain promotion, recorded independently. |
| `green_run_ids` | Ordered promoted `v3` `test-rust` run IDs. |
| `consecutive_green_count` | Derived from uninterrupted green run sequence. |
| `alias_removal_eligible` | True when at least one containing release exists. |
| `cargo_retirement_eligible` | True when consecutive green count is at least ten. |

The two eligibility values are independent. Either may become true first and
neither depends on the corresponding change having landed.

## Migration Lifecycle

```text
planned
  -> foundation-ready
  -> coverage-complete
  -> safety-complete
  -> shadowing
  -> evidence-qualified
  -> promoted
  -> release-qualified -> aliases-removed
  -> green-run-qualified -> cargo-retired
```

Transition rules:

- Each transition before promotion requires the prior wave merged and sealed
  by the unanimous ten-role panel. After promotion, each independent child
  transition requires W5 plus its own evidence gate and panel.
- `shadowing -> evidence-qualified` requires a valid Qualification Evidence
  Record.
- `evidence-qualified -> promoted` is the only executor-authority change.
- Before `cargo-retired`, rollback is one promotion revert because Cargo
  implementation still exists.
- `promoted -> release-qualified` requires a release containing promotion and
  is independent of the green-run clock and Cargo retirement.
- `promoted -> green-run-qualified` requires ten consecutive green promoted
  `v3` runs and is independent of release containment and alias
  removal.
- `release-qualified -> aliases-removed` removes only compatibility aliases.
- `green-run-qualified -> cargo-retired` removes only the eighteen Cargo
  implementations and unreachable Cargo-only plumbing. It must preserve the
  fixture mode and every public Make name, which continue to invoke the
  authoritative Bazel carriers.
- A failure before promotion remains in `shadowing`; it never weakens a gate.
- A promoted correctness failure reverts promotion and returns to
  `safety-complete` or `shadowing`, retaining evidence only as historical.

## Relationships

```text
Coverage Map 1 -- 18 Rust Surfaces
Rust Surface 1 -- 1..n Carrier Targets
Carrier Target 1 -- 1 Rust Surface
Carrier Target (TestCarrier) 1 -- 1 Test Topology
Carrier Target (TestCarrier) 1 -- 1 Per-Case Result Document
Carrier Target many -- 1 CI Slice
Injected Boundaries 2 -- many Carrier Targets and cleanup paths
Carrier Target 1 -- 1..n Verified Executable Handles
Test Locator Migration (Migrated, binary-location) 1 -- 1 Verified Executable Handle
Coverage Map 1 -- many Test Locator Migration records
Wave-Note Lint Refusal many -- 1 wave-note corpus
Hermeticity Inventory 4 -- 1 Coverage Map
Qualification Record many -- many protected-v3 push events
Seeded Failure Record 18 -- 1 Qualification Evidence Record
Performance Measurement Set 3 -- 1 Qualification Evidence Record
Cache Generation many -- 1 authorized writer policy
Recovery Condition many -- 1 owning safety subsystem
Qualification Evidence Record 1 -- 1 Promotion Record
Promotion Record 1 -- 1 Post-Promotion Observation
```
