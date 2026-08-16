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

Provider evidence must separate action-cache, CAS, output, stdout/stderr, BES,
repository, retry, provider-accounted transfer, and local U9 measurements.
Evidence is projected through `xtask buildbuddy-probe`; credential fields,
header-auth fields, bearer values, API-key values, and configured plain,
encoded, or split sentinels are rejected. Logs and BEP output are redacted
before they are retained.

No live BuildBuddy account run is part of this change. Live qualification
requires a protected credential-helper environment and sanitized provider
evidence; never place a real key in the repository or print it during a
probe.
