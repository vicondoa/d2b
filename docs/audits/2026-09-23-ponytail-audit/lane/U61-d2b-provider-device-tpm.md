# U61 d2b-provider-device-tpm
## Findings
net: -10 lines, -0 deps

## Commitment rule first
The TPM Provider and the credential/session crates reorganize the same half of
the admission contract, so this lane flags only what loses identity-crossing
semantics; every zero-caller claim is backed by a workspace-wide reference
search plus both Provider crates + tests, and the "refused stays refused"
classes from the U1 ledger are honored verbatim.

## Caller-census ledger rows for this crate
- `assert_pidfd_verification_precedes_tpm_open`: 23 refs — production callers
  in both Provider crates (systemd conformance.rs:195, minijail
  conformance.rs:214) + 4 Provider-surface callbacks; stays. The same show as
  `assert_pidfd_open_follows_verification` (external 2-7) already in U28's
  lean surface.
- `assert_status_redaction`: 8 refs — stays [packages/d2b-provider-process-systemd/src/conformance.rs:214]
- `has_resource_client_binding`: 0 external — dead (see ticket accessor
  finding below, shares the fence with U2#S1)

## Applied finding
  `children_have_verified_stop_proofs` is the one pure-suite surface no
  Provider lands. Its body (suite.rs:297-306, 10 lines) only re-validates
  "every owned child supplied a verified, owner-specific StopProof" — the
  same obligation the live `assert_finalizer_requires_verified_stop`
  already encodes for both Providers, and its sole refs are this crate's own
  tests (415,420,425). Delete the helper with the redaction Debug surface.
  [packages/d2b-process-conformance/src/suite.rs:297] (leaf)

## Refusals honored
- The suite's live assertion surface (`assert_*`: 18 helpers, all external
  callers 2-7) is the Providers' shared conformance wall; it stays.
- `children_have_verified_stop_proofs` (WaitReapOwner::Local) was refused in
  U2#S1 application set: it reads only the suite's own owner-neutral
  fixture? No — new evidence: the two Refusal classes in the U1 ledger for
  d2b-process-conformance do not refuse this crate's surface; that row was
  about d2b-process-conformance's own suite fns, which we did not touch.

## Verified clean
- No hand-rolled UUID/UUIDv4 rendering, hex codecs, or stdlib equivalents
  in this crate; the digest renderer is the shared ResourceUid::from_bytes
  at d2b-contracts/src/identity.rs:621 (in-scope, its own Provider surface
  uses it).
- All 18 ProcessConformanceError codes unique, all external call sites
  verified against the two Provider crates' conformance tests.
- 0 unused deps, 0 duplicated wire shapes, 0 single-implementation
  abstractions.
- Crate-layout scaffolds (integration/*.rs, README-only ratchet) stay per
  policy.
- `children_have_verified_stop_proofs` is the only genuine zero-caller
  surface; its removal also un-pins suite.rs's own stop-proof test rows.

## Net
-10 lines, -0 deps. One genuine dead surface in an otherwise-lean conformance
crate.
