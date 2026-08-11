Pass the accepted judgment and its bounded handoff to native
`compound-engineering.ce-work`.

`ce-work` is the sole editing lane in this expansion.  It may modify only the
assigned worktree and must run the targeted checks named by the judgment.  It
must not post, resolve, or otherwise mutate pull-request comments.  Preserve
the original comment and finding identifiers in the implementation summary.

If the judgment requests an operator decision, do not guess or edit around
that decision; return the durable decision request to the workflow.
