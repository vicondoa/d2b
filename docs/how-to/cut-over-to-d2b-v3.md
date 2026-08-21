# Cut over a daily-driver host to d2b v3

This runbook describes the one-time, host-wide cutover. It is deliberately
separate from scoped Zone, Provider, or Guest reset. The cutover lane stops at
`CutoverSucceeded`; merge, post-merge seal, and irreversible finalization
belong to U7.

## Before binding

Use one clean, committed candidate and one prebuilt closure. Freeze and record:

- candidate id, commit, tree, bundle generation, closure path, and GC root;
- the canonical, host-wide preview digest;
- the exact operator subject and host identity digests;
- the typed apply consent and its digest;
- the typed host-generation handoff;
- the strict external recovery-point attestation.

The recovery point must cover the complete host, every configured Zone in the
preview, boot and system state, preserved identity state, and the exact
restore instructions. It must be read-only through expiry and must have enough
remaining lifetime for apply, verification, guarded merge, and post-merge
seal. A local checkpoint or a copy under `/var/lib/d2b` is not the R19
recovery point.

Validate the candidate through the production delivery stages before any host
mutation:

```bash
bazel build //packages/xtask:xtask
bazel-bin/packages/xtask/xtask delivery wave recovery-import \
  --snapshot <candidate-snapshot> \
  --attestation <external-attestation> \
  --repo <logical-id>=<checkout-root> \
  --candidate-id <candidate-id> \
  --commit-oid <commit-oid> \
  --tree-oid <tree-oid> \
  --closure-store-path-sha256 <closure-digest> \
  --bundle-generation <bundle-generation> \
  --preview-sha256 <preview-digest> \
  --host-identity-sha256 <host-identity-digest> \
  --operator-subject-sha256 <operator-digest> \
  --restore-instructions-sha256 <restore-digest> \
  --recovery-point-locator-sha256 <locator-digest> \
  --required-remaining-ttl-seconds <seconds> \
  --verifier-now-unix <unix-seconds> \
  --command <qualified-verifier> \
  --state-dir <delivery-state>

bazel-bin/packages/xtask/xtask delivery wave seal \
  --snapshot <candidate-snapshot> \
  --repo <logical-id>=<checkout-root> \
  --state-dir <delivery-state>

bazel-bin/packages/xtask/xtask delivery wave merge-eligibility \
  --seal <candidate-seal> \
  --repo <logical-id>=<checkout-root> \
  --state-dir <delivery-state>
```

The validators own the allow/refuse predicates. Do not reproduce them in a
shell wrapper.

## Preview and consent

Run the preview without `--zone`. A one-time cutover is always host-wide:

```bash
d2b host cutover preview \
  --operation-id <operation-id> \
  --candidate-id <candidate-id> \
  --revision-plan-id <revision-plan-id> \
  --system-artifact-id <system-artifact-id> \
  --source-system-artifact-id <source-system-artifact-id> \
  --json
```

Confirm `state=planned`, `phase=0`, `mutationAccepted=false`,
`inventory.complete=true`, and that `inventory.zoneCount` equals the complete
configured Zone inventory. Repeat the preview and require the same digest.
Any missing, unexpected, duplicate, or incomplete Zone is a no-go.

The apply consent must be the canonical record bound to the exact operation,
candidate, preview, recovery digest, candidate artifact, preserved source
artifact, and operator. The apply handoff and preview must carry the same
candidate artifact identity. A rollback handoff must carry the admitted
preserved source artifact identity. A changed byte, stale digest, replayed
record, different operator, or different artifact is refused before bootstrap.

## Apply and observe

The production CLI admits the runner through `d2bd` and the broker before the
control plane is drained:

```bash
d2b host cutover apply \
  --operation-id <operation-id> \
  --candidate-id <candidate-id> \
  --revision-plan-id <revision-plan-id> \
  --source-system-artifact-id <source-system-artifact-id> \
  --preview-digest <preview-digest> \
  --recovery-digest <u3-recovery-digest> \
  --operator-id <bound-operator-id> \
  --consent-digest <consent-digest> \
  --consent-file <u3-consent-json> \
  --recovery-attestation-file <u3-recovery-json> \
  --host-digest <host-digest> \
  --handoff-file <typed-host-generation-handoff> \
  --json
```

The U3 recovery JSON is the compact operation-bound record consumed by the
cutover engine. The U5 attestation is the larger delivery record consumed by
`recovery-import`; both must bind the same candidate, preview, host, operator,
and restore instructions.

Use the runner socket while the daemon is drained:

```bash
d2b host cutover status --operation-id <operation-id> --json
d2b host cutover hold --operation-id <operation-id> --reason "<bounded reason>" --json
d2b host cutover resume --operation-id <operation-id> --json
d2b host cutover doctor --operation-id <operation-id> --json
```

`status` is redaction-safe. `held` means no new effect starts; an in-flight
atomic effect may finish first. Any configured Admin may set a hold. Only the
bound operator may resume without fresh consent. `doctor` must not report
`cutover-quarantined`, `adoption-quarantined`, unresolved ownership, or
recovery degradation.

The runner owns one durable journal and one host-wide OFD lock. A daemon or
client restart must leave the runner active and must not transfer repair
ownership to `d2bd`.

## Rollback and external restore

The rollback boundary is phase 4, after disposition staging and before the
resource store is committed:

```bash
d2b host cutover rollback \
  --operation-id <operation-id> \
  --handoff-file <typed-host-generation-handoff> \
  --json
```

Through phase 4, native rollback restores the preserved source generation,
keeps TPM, durable Volume, store-view, SSH-key, and audit bytes intact, and
quarantines staged destinations. It is not a filesystem-wide transaction.

At phase 5 (`resource-store`) and later, native rollback is refused. A
failure, expiry, ambiguous destination, or candidate mismatch becomes
`RestoreRequired`. Stop issuing cutover commands and use only the qualified
external recovery mechanism. Do not attempt a replacement candidate,
finalization, or a second native rollback. Record the restore result and
re-run the same identity and audit digest checks after the host is booted.

The VM rehearsal must exercise this phase-5 failure class with the same
recovery mechanism used by the operator. The daily-driver attestation binds
only digests of the locator and restore instructions; it must never contain a
raw URL, mount path, hostname, uid, operator name, credential, token, or
backup payload.

## Verify and stop

After all typed effects complete, provide observations for every configured
Zone:

```bash
d2b host cutover verify \
  --operation-id <operation-id> \
  --verification-file <all-zone-verification-json> \
  --json

d2b host cutover doctor --operation-id <operation-id> --json
d2b host cutover status --operation-id <operation-id> --json
```

Verification must prove:

- the observed Zone set exactly equals the configured Zone set;
- every Zone is healthy;
- preserved sources and identity digests match;
- the candidate is still current; and
- the broker audit publication is durable and continuous.

The required stop state is `CutoverSucceeded`. Do not invoke
`d2b host cutover finalize` in this lane.

## Finalization is separate

Only after the guarded merge, post-merge reconciliation, post-merge seal,
fresh status/verify/doctor checks, and a new operator decision may U7 issue the
phase-10 consent:

```bash
d2b host cutover finalize \
  --operation-id <operation-id> \
  --consent-file <separate-finalization-consent> \
  --finalization-file <approved-dispositions> \
  --json
```

The phase-10 consent is bound to a different finalization binding. Apply
consent cannot authorize it. Until that second consent is accepted, all
legacy sources remain preserved and available to the external restore path.

## Redaction rules

Keep operator logs limited to fixed labels, stable status classes, phase
numbers, and digest values intended for the candidate record. Suppress command
arguments and raw validator output from the live wrapper. Never print:

- candidate state roots, store paths, sockets, hostnames, or process ids;
- recovery locators, restore text, backup names, or provider credentials;
- usernames, UIDs, SSH key material, tokens, or realm principals; or
- unredacted journal, audit, support-bundle, or terminal output.

Use synthetic RFC1918/RFC5737 examples in rehearsal evidence and describe
redacted failures by their typed class rather than copying raw diagnostics.
