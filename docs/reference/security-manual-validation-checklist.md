# d2b v3 security manual validation checklist

This checklist is the human evidence companion for the destructive U6
cutover lane. It does not replace the production U3 engine, U4 broker
boundary, U5 delivery validators, or the booted-VM rehearsal.

## Candidate and recovery binding

- [ ] The candidate is committed, clean, frozen, and its commit, tree, bundle
      generation, closure digest, and protected GC root are recorded.
- [ ] The preview was produced by `d2b host cutover preview` without `--zone`.
- [ ] The preview is byte-stable on repeat and `inventory.complete=true`.
- [ ] `inventory.zoneCount` equals every configured Zone on the host.
- [ ] The candidate snapshot, seal, recovery attestation, consent, handoff, and
      verification files are the exact intended files.
- [ ] `recovery-import`, `seal`, and `merge-eligibility` passed on that
      candidate; no validator result was copied from another candidate.
- [ ] The external recovery point is full-host, same-host, read-only through
      expiry, and covers boot/system state, every previewed artifact, preserved
      identity state, and the exact restore instructions.
- [ ] The external provider's readback or backup verification passed.
- [ ] The attestation has enough remaining lifetime for apply, verify, guarded
      merge, and post-merge seal.
- [ ] The local checkpoint is treated as evidence only, never as the R19
      recovery point.

## Consent and admission

- [ ] Apply consent is canonical, single-use, and bound to the exact
      operation, candidate, preview, recovery digest, and operator.
- [ ] A wrong candidate, tree, closure, preview, host, operator, restore
      instruction digest, or recovery locator fails before mutation.
- [ ] A stale, expired, replayed, duplicate, fractional, negative, or
      unknown-field record fails closed.
- [ ] The CLI is run as the bound Admin principal; no Launcher, shutdown hook,
      or unrelated Admin is substituted.
- [ ] The broker admitted one operation-scoped bootstrap capability and the
      runner consumed it once.
- [ ] The runner is outside `d2b.slice`, owns the journal and OFD lock, and
      no persistent cutover systemd unit was installed.

## Drain, restart, and typed effects

- [ ] The runner remains reachable through its owner-authenticated socket after
      the client disconnects.
- [ ] Any configured Admin can set a safety hold.
- [ ] Only the bound operator can resume without fresh consent; a non-owner
      resume without its digest is refused.
- [ ] The runner drains `d2bd` only after the journal records the transition.
- [ ] The adapted broker remains the only privileged effect path after drain.
- [ ] The typed host-generation handoff names an authenticated Host target,
      generation ancestry, compatibility floor, and catalog artifact. It
      contains no path or command.
- [ ] Each effect is in the closed cutover allowlist, has the expected phase
      and replay class, and publishes durable audit evidence before success.
- [ ] A daemon restart reattaches read-only observation and never repairs or
      adopts the runner journal.

## Rollback and restore boundary

- [ ] A phase-0 through phase-4 interruption rolls back through the preserved
      source generation, quarantines staged destinations, and leaves TPM,
      durable Volumes, store-view, SSH keys, and audit bytes unchanged.
- [ ] A phase-5-or-later interruption refuses native rollback and reports
      `RestoreRequired`.
- [ ] The exact external recovery mechanism was exercised in the VM rehearsal
      after a phase-5-or-later failure.
- [ ] The operator's restore instructions were independently verified and are
      bound by digest.
- [ ] After the drill, identity and audit digests match the recovery point.
- [ ] No replacement candidate, native rollback, or finalization was attempted
      after the external-restore boundary.

## Verification and finalization

- [ ] `d2b host cutover verify` observes exactly the configured Zone set.
- [ ] Every Zone is healthy and the candidate remains current.
- [ ] Preserved sources and identity digests match.
- [ ] Audit continuity is durable across bootstrap, hold/resume, drain,
      generation handoff, effect completion, and verification.
- [ ] `d2b host cutover doctor` reports no cutover or adoption quarantine and
      no unresolved ownership or recovery degradation.
- [ ] The observed stop state is `CutoverSucceeded`.
- [ ] The U6 lane did not invoke `d2b host cutover finalize`.
- [ ] U7 obtained a separate digest-bound finalization consent only after
      guarded merge, post-merge reconciliation, post-merge seal, and fresh
      status/verify/doctor checks.
- [ ] The finalization disposition contains only approved operation-owned
      artifacts; the active closure, recovery material, journal, store-view
      roots, and audit segments remain protected.

## Redaction evidence

- [ ] Logs contain fixed labels, typed failure classes, phase numbers, and
      approved digests only.
- [ ] No raw host path, store path, socket, hostname, PID, UID, username,
      recovery locator, restore text, credential, token, SSH key, or realm
      principal appears in logs, audit fields, CLI errors, or support output.
- [ ] Recovery evidence stores only the canonical attestation digest and
      opaque locator digest; the recovery payload remains with the external
      provider.
- [ ] Any screenshot or terminal capture was redacted before retention.
- [ ] Rehearsal examples use generic identities and RFC1918/RFC5737 ranges,
      never daily-driver host data.
