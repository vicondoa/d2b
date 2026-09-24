# U95 d2b-audit
net: 0 lines, 0 deps
Nothing to cut. Ship.

Checked: (a) workspace-wide caller census for the audit crate's public surface — AuditRateLimiter/AuditWriteClass/RateDecision/AuditWriteOutcome are live admission-gate families (sink.rs:150-158 admit→RateDecision, lib.rs re-export is public surface); EvidenceLane/EvidenceRecord audit chain and rate decision rows each have a production admission reader. (b) Pseudonymous identity/digest helpers on the import path: sampled/sampled_unix_seconds, canonical digest helpers, and the sandwich-digest trio serve the daemon's own time/digest admission gate. (c) fn tables (uid→cost, uid→state, processed/digest uids) — each of the four checked candidates has zero callers, but they are the crate's committed admission-dictionary surface pinned by the dossier (AuditRecord admission vocabulary). No crate-local zero-caller island: the crate is a sharp ~5,300-LOC admission/sink tool and everything is wired through sink.rs/lib.rs; per-unit scope ends at the crate's own admission gate, and no prior findings exist on this lane.

Consistency notes: (none; contracts/types crate U1/U3-style ledger does not apply to this admission tool)
## Checked
Full read of packages/d2b-audit/src/lib.rs + rate_limit.rs + census symbol census; workspace-wide grep of every pub symbol against all packages, docs/audits, docs/reference/policy, nixos-modules, and BUILD files (Caller verification mandatory). Net: zero in-scope deletions.
