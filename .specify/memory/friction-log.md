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
| spec-copilot-w3 | build | 2026-07-31 | `specify init` replaces `installed_integrations` rather than appending, silently dropping a coexisting install | 1 | resolved |
| spec-copilot-w3 | build | 2026-07-31 | `specify init` rewrites shared `.specify/scripts` and `.specify/templates`, reintroducing banned dash codepoints into tracked files | 1 | open |
| spec-copilot-w3 | codegen | 2026-07-31 | Shipped spec-kit skill text carries banned dash codepoints and must be de-dashed on every import | 1 | open |
| spec-copilot-w3 | codegen | 2026-07-31 | Delivery help output is pinned by a wire-fingerprint golden, so even a documentation-only field needs a schema bump that would invalidate in-flight artifacts | 1 | resolved |
| spec-copilot-w6 | signoff | 2026-07-31 | A reviewing lane's self-reported model is confabulated: five of ten seats named a model other than the one the harness dispatched, so a self-report tripwire cannot detect a mis-dispatch | 1 | resolved |
| spec-copilot-w6 | test | 2026-07-31 | `make test-fixture-contracts` fails closed without `D2B_ENABLE_FIXTURE_BUILD=1`, which is set in the job manifest but not in the bare target, so a hand-run of the lane looks broken | 1 | open |
| spec-copilot-w6 | build | 2026-07-31 | `make X 2>&1 \| tail` reports the pager's exit status, so a piped gate invocation can read as passing when it failed | 1 | open |
