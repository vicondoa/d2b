Finalize the separated resolution only after the review judgment, native
`ce-work` edit, read-only verifier, and native Compound Engineering synthesis
have completed.

Run the inherited final-report artifact check.  This stage records the
workflow result and leaves pull-request publication and merging to the
managed operator controls.

After the artifact check, perform the deterministic terminal handoff.  The
helper re-reads the authoritative root bead and writes a terminal record only
when that bead is terminal.  Cancellation, an open pull request, a nonterminal
bead, or a missing authoritative state leaves the active-run roots in place:

```bash
python3 "$GC_CONTRIBUTOR_ROOT/pack/scripts/service-activation.py" \
  write-terminal-state \
  --terminal-state-root "$GC_TERMINAL_STATE_ROOT" \
  --run-id "$GC_RUN_ID" \
  --bead-id "$GC_ROOT_BEAD_ID" \
  --generation "$GC_CITY_GENERATION" \
  --state-schema "$GC_STATE_SCHEMA" \
  --bd-path bd \
  --cancellation-root "$GC_CANCEL_ROOT"
```

Never synthesize or edit the terminal JSON directly.  Re-running this stage
with the same terminal bead is idempotent; stale or forged records fail closed.
