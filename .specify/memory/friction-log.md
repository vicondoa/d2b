# Friction log

The engineering setup getting in the way: a gate that is slow or flaky, a
command that has to be rediscovered, a step that is easy to get wrong, a
failure whose message did not say what to do.

Managed by the `d2b-memory` skill. **Classification metadata only.**

Record at the moment it happens, not at the end of the wave. Friction noticed
during a fix round and not written down is the single most commonly lost
observation in this process.

**The escalation rule.** A category recurring across three waves stops being
friction and becomes a task. That is a count, not a judgement: when the third
row lands, it gets promoted into the plan.

| Wave | Category | Date | Statement | Recurrence | Disposition |
|---|---|---|---|---|---|
| copilotw3 | build | 2026-07-31 | `specify init` replaces `installed_integrations` rather than appending, silently dropping a coexisting install | 1 | resolved |
| copilotw3 | build | 2026-07-31 | `specify init` rewrites shared `.specify/scripts` and `.specify/templates`, reintroducing banned dash codepoints into tracked files | 1 | open |
| copilotw3 | codegen | 2026-07-31 | Shipped spec-kit skill text carries banned dash codepoints and must be de-dashed on every import | 1 | open |
| copilotw3 | codegen | 2026-07-31 | Delivery help output is pinned by a wire-fingerprint golden, so even a documentation-only field needs a schema bump that would invalidate in-flight artifacts | 1 | resolved |
| copilotw6 | signoff | 2026-07-31 | A reviewing lane's self-reported model is confabulated: five of ten seats named a model other than the one the harness dispatched, so a self-report tripwire cannot detect a mis-dispatch | 1 | resolved |
| copilotw6 | test | 2026-07-31 | `make test-fixture-contracts` fails closed without `D2B_ENABLE_FIXTURE_BUILD=1`, which is set in the job manifest but not in the bare target, so a hand-run of the lane looks broken | 1 | open |
| copilotw6 | build | 2026-07-31 | `make X 2>&1 \| tail` reports the pager's exit status, so a piped gate invocation can read as passing when it failed | 1 | open |
| copilotw6fu2 | test | 2026-07-31 | The qualified wave grammar was enforced for delivery state paths and fold targets but not for the memory registers' own wave column, so this branch wrote nine illegal hyphenated tokens into its own registers | 1 | resolved |
| copilotw6fu2 | signoff | 2026-07-31 | A malformed commit trailing tag is detectable only by a reviewer, so correcting it needs a branch-wide history rewrite rather than one amend | 1 | open |
| copilotw6fu2 | build | 2026-07-31 | A Layer-1 job killed by an external signal reports `exit -15` and fails the whole gate, and the phase summary does not distinguish that from a real defect without reading the retained tail | 1 | open |
| copilotw6fu15 | signoff | 2026-07-31 | A panel lane dispatched through the generic subagent type keeps its shell despite the seat's read-only prompt, and one wrote probe files into the repository root during a round | 1 | open |
| copilotw6fu15 | signoff | 2026-07-31 | A seat returns an empty response and casts no vote unless its prompt explicitly forbids it; it happened to three seats across the rounds and is silent unless the tally is counted | 3 | open |
| copilotw6fu15 | signoff | 2026-07-31 | A prompt mandate to audit an entire class of diagnostic produced a finding naming four sites that already complied, because the seat read only each message's first clause | 1 | resolved |
| copilotw6fu15 | signoff | 2026-07-31 | Two seats asked to delete a check two other seats had just defended; the disagreement was only resolvable by evidence from a third file, not by weighing the seats | 1 | resolved |
| copilotw6fu15 | test | 2026-07-31 | A coverage case cannot discriminate against the prior commit, so proving one is not vacuous needs a deliberate gate mutation and a restore, which no target automates | 2 | open |
| copilotw6fu15 | disk | 2026-07-31 | Copying the packages tree to build an out-of-tree probe pulls the multi-gigabyte cargo target with it and consumed 34 GB before it was stopped | 1 | resolved |
| copilotw6fu17 | merge | 2026-07-31 | A move-versus-edit conflict cannot be resolved from diff3 markers alone, because the other side deleting a passage and rewriting it produce an identical hunk; extracting the other side's own diff first is what makes it tractable | 1 | resolved |
| copilotw6fu17 | signoff | 2026-07-31 | Grepping a distinctive phrase from every incoming addition proves presence, not completeness, and no gate in this repo detects a rule silently dropped from prose, so a merge of prose is one of the few changes where the panel round is the only real check | 1 | wontfix |
| copilotw6fu17 | signoff | 2026-07-31 | A finding can be factually true about the tree and wrong about attribution; per-file comparison against the upstream branch converted a twelve-file sweep into a one-row register entry and the seat withdrew it | 2 | resolved |
| copilotw6fu18 | signoff | 2026-07-31 | A finding that asks for evidence rather than a change closes without a commit, so the round can be answered by running the gate; the panel rule needed no exception for this because the tree does not move | 1 | resolved |
| copilotw6fu19 | signoff | 2026-07-31 | Recording a closed round reopens it, which looked non-terminating; a skill carve-out was written to answer that and was reverted in round 20 | 1 | wontfix |
| copilotw6fu20 | signoff | 2026-07-31 | Recording a closed round reopens it, which looks non-terminating but is not; the loop closes by the same fixed point as a fix round, because an accurate record generates no new friction and there is then nothing left to write | 1 | resolved |
| copilotw6fu20 | signoff | 2026-07-31 | A proposed exemption to the panel gate was rejected by three seats on independent grounds; loosening a gate is the case where the panel earns its cost, so propose the exemption rather than take it | 1 | resolved |
| copilotw6fu20 | signoff | 2026-07-31 | A memory register row is a commitment and not only a history, so an unreviewed append can defer a fix the panel never agreed to or mark a requirement wontfix | 1 | resolved |
| copilotw6fu20 | signoff | 2026-07-31 | An exemption condition that cannot be checked mechanically is an honour-system condition and does not bound the exemption it appears to bound | 1 | resolved |
| copilotw6fu20 | signoff | 2026-07-31 | The panel invalidation rule is stated in four separate files, so amending one silently contradicts three and nothing detects the divergence | 1 | open |
| copilotw6fu21 | signoff | 2026-07-31 | The agent tool's default working directory is a different worktree, and `&` detaches the leading `cd`, so a diff staged for panel review was computed in the wrong checkout and looked plausible rather than failing | 1 | resolved |
| copilotw6fu22 | signoff | 2026-07-31 | Two seats independently raising the same finding has been right every time it happened in this phase, so it is worth treating as near-certain signal rather than re-litigating | 4 | resolved |
| copilotw6fu23 | signoff | 2026-07-31 | The panel contract requires each round to stage both a delta and a full-branch diff, and a seat reviewed the full one while reporting on the delta, so the two-file layout is itself a source of misattribution | 1 | resolved |
| copilotw6fu24 | signoff | 2026-07-31 | A category was corrected to answer one finding without being checked against a row added in the same commit describing the same hazard, so the fix left two adjacent rows disagreeing and the closed vocabulary is validated per row but never across rows | 2 | resolved |
| copilotw6fu25 | signoff | 2026-07-31 | Panel lanes are read-only by construction, which is what stops them stampeding the shared store, but it also means no seat can check an external precondition, and a shipped command depending on a repository label that does not exist passed twenty-five rounds | 1 | resolved |
