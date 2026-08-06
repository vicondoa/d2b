# Cache and Workflow Permission Boundaries

## Shadow

- The workflow is non-required and outside `V3_PR_GATE_WORKFLOWS`.
- It restores and saves no Bazel cache.
- Pull-request runs remain path-filtered and diagnostic.
- Pull-request jobs request only `contents: read` and no `actions: write`.
- No direct, indirect, post-step, or unknown cache writer is reachable.
- Checkout uses `persist-credentials: false`.
- Cache credentials never enter `run:` or a Bazel environment.

## Promoted cache kinds

| Kind | Maximum |
| --- | ---: |
| Action | 4 GiB |
| Repository/download | 1 GiB |

The output base is never cached.

Every protected-`v3` qualification record carries three explicit nonnegative
counts:

- `bazelRestoreCount`;
- `bazelSaveCount`;
- `bazelPublicationCount`.

Every record carries all three. Shadow records require all three to be zero. A
cold continuous-integration record additionally carries `sliceDurationsSeconds`
with exactly four completed durations and requires
`bazelRestoreCount == 0`. These four camelCase names are the canonical
spellings everywhere. A missing count or duration is a refusal and is never
interpreted as zero.

## Bound key inputs

The table is authoritative. `A` means the action-cache key changes; `R` means
the repository/download-cache key changes. A dash means the input cannot
change that cache's fetched-byte identity and need not invalidate it.

| Bound input | A | R |
| --- | :---: | :---: |
| `.bazelversion` | A | R |
| `MODULE.bazel` | A | R |
| `MODULE.bazel.lock` | A | R |
| `.bazelrc` | A | R |
| stable Rust toolchain pin | A | R |
| nightly Rust toolchain pin | A | R |
| `packages/Cargo.lock` | A | R |
| walker `Cargo.lock` | A | R |
| `packages/Cargo.guest.lock` | A | R |
| `bazel/cargo/product.lock` | A | R |
| `bazel/cargo/walker.lock` | A | R |
| `cargo-bazel` URL | A | R |
| `cargo-bazel` sha256 | A | R |
| root deny configuration | A | R |
| broker deny configuration | A | R |
| guest deny configuration | A | R |
| pinned RustSec database revision and hash | A | R |
| yanked snapshot bytes | A | - |
| generated package-policy inputs | A | - |
| package-policy system/target mapping digest | A | - |
| selected-source census rules | A | R |
| selected-source checksum rules | A | R |
| `.bazelignore` | A | - |
| absolute startup-option shape | A | - |
| symlink prefix | A | - |
| build-script annotations | A | - |
| action-environment allowlist | A | - |
| seccomp syscall-policy digest | A | - |
| Bazel 8.6.0 upstream source digest | A | R |
| Linux sandbox seccomp patch digest | A | R |
| patched Bazel output NAR, executable, and capability-ABI digests | A | R |
| immutable execveat-helper source, selected dependency, output NAR, and executable digests | A | R |
| stable/nightly action-kind and sandbox-strategy coverage digest | A | - |
| generated BUILD digest | A | - |
| configured native-target digest | A | - |
| native runner architecture and exact system/target mapping | A | R |

A table-driven test mutates each row independently and proves every marked
primary key and restore prefix changes. For a row marked only `A`, the test
also proves the repository namespace remains well formed but does not require
needless invalidation.
Every case asserts the action and repository namespaces have different fixed
kind components; changing an input may never collapse the two keys or prefixes
into one namespace.

The primary key for each cache kind includes a successful protected-`v3` run
ID and is unique for that run. Restore prefixes omit both the run ID and the
commit SHA but bind the same applicable semantic-input digest as their
primary key. A prefix containing either run identifier is invalid because it
cannot find a prior generation. Action and repository entries use distinct
namespaces and never share a key.

## Native architecture lanes

X86 and arm jobs never restore each other's system-specific package-policy or
Nix realization cache under the same key. Each native lane binds its runner
architecture and exact system-and-target input mapping.

Neither lane sets a foreign system, `--builders`, or a remote builder.

## Trimming and maintenance

`packages/xtask/src/bazel_cache_contract.rs` owns a closed typed prefix enum
whose committed values are the retired Cargo cache prefixes and the action and
repository Bazel prefixes. The workflow, API response, command line, record,
and caller cannot add a prefix. Each cache entry is classified against exactly
one enum value before retention is evaluated. An unknown, caller-supplied, or
multiply matching prefix is preserved and reported as an authorization
refusal; it is never adopted as a new generation and never deleted.

1. Stop retired Cargo cache writes at promotion.
2. Enumerate all cache pages.
3. Refuse failed, incomplete, or ambiguous enumeration.
4. Classify every entry through the closed committed prefix enum and delete
   only authorized retired or superseded generations.
   For each authorized Bazel prefix, retain the newest complete generation and
   delete only older generations beyond retention. Completion order, not
   lexical run-ID order, selects the newest generation.
5. Run and await the synchronous on-demand collector.
6. Require repository use plus planned snapshots at most 8 GiB.
7. Recheck immediately before save.
8. Publish from one protected-`v3` writer.

The maintenance verdict is independent of the Rust verdict. Pull requests
restore read-only after promotion.

Fixtures cover duplicate primary keys, a primary key without the successful
run ID, a restore prefix containing a run ID or commit SHA, cross-kind key
reuse, deletion of the newest generation, retention of an older generation
instead of the newest, a missing `bazelRestoreCount`, `bazelSaveCount`,
`bazelPublicationCount`, or `sliceDurationsSeconds` entry, and every row
of the bound-input table. A missing row, a mutation that leaves an applicable
key unchanged, or equal action/repository namespaces fails.

Pagination fixtures interleave authorized retired, authorized current,
authorized superseded, unknown, caller-supplied, and ambiguously matching
entries across at least three page boundaries. The positive result deletes
only the authorized retired and superseded entries, preserves every unknown
or unauthorized entry byte-for-byte, and retains the newest complete
generation. Separate mutations drop the middle page, repeat a cursor, move an
unauthorized entry between pages, supply a prefix through the record, and make
one entry match two enum variants. Each refuses before any delete call; a
recording API proves the delete-call list is empty on refusal.

Page tokens and cursors exist only inside the injected API session. No cache,
qualification, post-promotion, diagnostic, or validator artifact persists or
prints them. Persisted completion is the closed state `complete` plus page and
entry counts and a complete-stream SHA-256. Pagination failures render a fixed
code, the repository-relative cache-policy row, and the stream digest if one
was completed; they never include `$!`, an absolute or Nix store path, cache
entry path, raw cursor, or opaque API handle.

## Policy fixtures

Fixtures reject direct and post-step cache saves, unknown writers, pull-request
`actions: write`, missing promoted deadlines, cross-architecture cache keys,
foreign-system arguments, and remote-builder arguments. A restore-only pull
request and one protected-`v3` writer pass.
