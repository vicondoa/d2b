# Bazel and BuildBuddy

This repository keeps Bazel's complete target set and uses BuildBuddy only
through the typed `tests/tools/bazel-check` facade. The facade owns the U9
evidence gate, credential policy, redaction, and the one permitted local
retry.

## Profiles

The committed `.bazelrc` defines `common`, `local`, `remote`, `trusted-seed`,
and `qualification` over the same `//...` target set. Remote profiles use:

- Bazel 9.2 credential-helper authentication only;
- `--remote_download_outputs=minimal`;
- zero Bazel remote retries, because the facade owns fallback;
- the immutable `d2b-bazel-worker/v1` platform contract; and
- separate developer, trusted-seed, and qualification instance namespaces.

Header authentication is forbidden. Do not add `--remote_header`,
`--bes_header`, API keys, bearer values, or provider-specific experimental
remote flags to repository configuration. The Gas City BuildBuddy proxy is a
separate service and is not part of these profiles.

Copy `.bazelrc.user.example` to `.bazelrc.user` only for a private local
credential-helper setup. Store the key in the protected file named by
`D2B_BUILDBUDDY_CREDENTIAL_FILE` (default:
`~/.config/d2b/buildbuddy-api-key`). The key is read by the helper and is
never an argument, repository rule input, action environment value, platform
property, BEP field, or checked-in evidence field.

## U9 gate

Every non-local profile first validates
`tests/golden/bazel/cache-transfer-representative.json` against the committed
eligibility digest and cache policy. The current representative bounds are:

- 207 actions;
- 162901404939 gross input bytes;
- 1034798612 unique input bytes; and
- pipelining rejected because it increases gross inputs and fan-out.

The graph, configuration, platform, toolchain, and pipelining values must also
match the policy. A missing, stale, or mismatched report blocks all remote
profiles. The local transfer command remains credential-free:

```bash
make bazel-cache-transfer-report
```

## Running the facade

Use the pinned Bazel provider in the Bazel Nix shell:

```bash
nix develop .#bazel
tests/tools/bazel-check --profile local
tests/tools/bazel-check --profile remote
```

The facade always supplies `--repo_contents_cache=` as a local command option.
Repository-content caching is not a remote profile feature. It runs the
identical target set locally when credentials are unavailable, when an
untrusted job requests a remote profile, or when trusted injection is not
authorized.

Only these pre-dispatch infrastructure classes permit one local retry:
missing credentials, authentication, endpoint, worker, and transport. A
post-dispatch uncertainty, analysis failure, policy failure, test failure, or
ordinary build failure is fail-closed and is never retried locally.

## Trusted injection

Untrusted pull-request jobs receive no BuildBuddy credential and use the local
profile. Trusted profiles require all of:

1. `D2B_BAZEL_TRUSTED=1`;
2. `GITHUB_REF=refs/heads/v3`; and
3. a security digest allowlisted by `tests/golden/bazel/cache-policy.json`.

The digest covers the Bazel security configuration, module lock, platform and
remote policy, and this facade. A change to an endpoint, instance, module
source, repository rule, action environment, or security file therefore
withholds trusted credentials until the digest is explicitly reviewed and
allowlisted.

The same protected-ref and digest checks apply to any remote profile running in
GitHub Actions, not only the trusted-seed and qualification profiles.

Trusted seeds write only to the trusted namespace. Developer and qualification
namespaces are partitioned by trust, architecture, toolchain, platform,
worker-image contract, feature set, and lock. Branch and commit names do not
create cache namespaces.

## Evidence and sentinels

Provider evidence must be emitted by the credential-helper probe for the
one-run nonce and bind the invocation, sample, commit, worker image,
fresh-worktree provenance, and candidate identity. It must separate action-cache,
CAS, output, stdout/stderr, BES, repository, retry, provider-accounted
transfer, and local U9 measurements.
Evidence is projected through `xtask buildbuddy-probe`; credential fields,
header-auth fields, bearer values, API-key values, and configured plain,
encoded, or split sentinels are rejected. Logs and BEP output are redacted
before they are retained.
The projection is not a provider attestation. Caller-authored candidate or
provider JSON is therefore quarantined as non-qualifying outside the explicit
sanitized test-fixture mode until a provider attestation or generated evidence
collector establishes the production origin.

No live BuildBuddy account run is part of this change. Live qualification
requires a protected credential-helper environment and sanitized provider
evidence. The qualification wrapper owns the one-run nonce, passes it to the
provider evidence output path, and refuses a dirty worktree. If the live
attempt does not produce a complete provider-accounted record, the wrapper
emits a non-qualifying report rather than inventing metrics. Never place a
real key in the repository or print it during a probe.

## Qualification evidence

U7 keeps scheduler replacement fail-closed. The qualification command consumes
local candidate evidence plus a sanitized provider observation:

```bash
tests/tools/buildbuddy-qualification \
  --mode acceptance \
  --candidate .scratch/bazel-qualification/candidate.json \
  --provider-evidence .scratch/bazel-qualification/provider.json \
  --output .scratch/bazel-qualification/report.json
```

The report binds the current commit, sorted target-set digest, committed
configuration digest, selected-closure digest, qualification namespace, and
toolchain. It also records coverage parity, trusted-seed completion,
unchanged-cache behavior, typed fallback, action-cache/CAS/BES/repository and
retry traffic, local Nix time, provider-accounted upload and download, worker
identity, and fresh-worktree wall-time distributions.
Qualification runs reject an ignored workspace `.bazelrc.user` and disable
system and home Bazel rc files so the committed `.bazelrc` digest is the
effective configuration binding.

Provider transfer is the sum of uploaded and downloaded bytes. The monthly
projection is `floor(80,000,000,000 / P99)` and leaves 20,000,000,000 bytes of
the stated allowance as headroom. Qualification requires five independent
fresh-worktree samples. Upload and download distributions are checked against
the U9 input and output bounds before the combined transfer comparison; the U9
representative report supplies the pessimistic upper bound, and material
divergence is reported rather than silently rewriting the local model. The
canonical U9 report is also pinned to the qualification target-set and
configuration digests; either change invalidates the bounds until a new
representative report is accepted.

Missing or incomplete provider-accounted transfer produces
`"status": "non-qualifying"` with null transfer percentiles and monthly runs.
The command never replaces missing metrics with zero; a valid P99 may still
publish its projection when another qualification reason blocks the result.
Stale, duplicate,
replayed, forged, path-bearing, secret-bearing, client-supplied, and
cross-commit evidence is rejected. Only the five typed pre-dispatch
infrastructure classes may receive one local retry; post-dispatch uncertainty
and product failures remain fail-closed.

Qualification is evidence gathering, not the U8 cutover. The current Make and
CI scheduler remains authoritative until a complete sanitized report and the
remaining cutover gates are accepted.
