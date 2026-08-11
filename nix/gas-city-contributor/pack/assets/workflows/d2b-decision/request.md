Use the private outbound Discord sidecar.  Do not read a Discord credential,
open a public listener, call an interaction endpoint, or write an approval
record.

```bash
PROMPT_JSON=$(python3 "$GC_CONTRIBUTOR_ROOT/pack/scripts/discord-decision.py" request \
  --socket "$GC_DISCORD_CHANNEL_SOCKET" \
  --run-id "$GC_RUN_ID" \
  --bead-id "$GC_DECISION_BEAD_ID" \
  --decision-id "$GC_DECISION_ID" \
  --prompt-nonce "$GC_DECISION_NONCE" \
  --assignee "$GC_DECISION_ASSIGNEE" \
  --guild-id "$GC_DISCORD_GUILD_ID" \
  --channel-id "$GC_DISCORD_CHANNEL_ID" \
  --message "$GC_DECISION_MESSAGE" \
  --choices-json "$GC_DECISION_CHOICES_JSON")
```

Persist the returned `prompt_nonce`, `message_id`, `run_id`, and
`decision_id` on the gate bead before waiting:

```bash
bd update "$GC_DECISION_BEAD_ID" \
  --set-metadata \
    "decision_run_id=$GC_RUN_ID" \
    "decision_id=$GC_DECISION_ID" \
    "decision_nonce=$(printf '%s' "$PROMPT_JSON" | jq -r '.prompt_nonce')" \
    "decision_message_id=$(printf '%s' "$PROMPT_JSON" | jq -r '.message_id')"
```

The nonce and message id are correlation data for restart reconciliation; they
are not an approval record or evidence artifact.

If the sidecar reports an already waiting or answered prompt with the same
identity, reuse it.  A different nonce or channel is a conflicting request
and must stop without sending another prompt.
