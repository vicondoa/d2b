Read the durable prompt through the private sidecar and wait for one
validated answer.  The sidecar must reject wrong guilds, channels, operators,
reply targets, run ids, decision ids, nonces, edits, stale prompts, orphan
replies, malformed choices, and duplicate/conflicting events before this
stage receives an event.

```bash
EVENT_JSON=$(python3 "$GC_CONTRIBUTOR_ROOT/pack/scripts/discord-decision.py" wait \
  --socket "$GC_DISCORD_CHANNEL_SOCKET" \
  --run-id "$GC_RUN_ID" \
  --decision-id "$GC_DECISION_ID" \
  --timeout 300)
```

Extract the returned `event_id` and `choice`, then make the gate bead the
authority for the first answer.  The update must include every correlation
field and both beads compare-and-set guards:

```bash
python3 - "$EVENT_JSON" <<'PY'
import json
import sys

event = json.loads(sys.argv[1])
if event.get("router_status") not in {"pending", "answered", "closed"}:
    raise SystemExit("decision is not ready")
if not event.get("event_id") or not event.get("choice"):
    raise SystemExit("decision correlation is incomplete")
PY

set +e
bd update "$GC_DECISION_BEAD_ID" \
  --if-assignee "$GC_DECISION_ASSIGNEE" \
  --if-status blocked \
  --status in_progress \
  --set-metadata \
    "decision_run_id=$GC_RUN_ID" \
    "decision_id=$GC_DECISION_ID" \
    "decision_nonce=$GC_DECISION_NONCE" \
    "decision_message_id=$(printf '%s' "$EVENT_JSON" | jq -r '.message_id')" \
    "decision_event_id=$(printf '%s' "$EVENT_JSON" | jq -r '.event_id')" \
    "decision_choice=$(printf '%s' "$EVENT_JSON" | jq -r '.choice')"
STATUS=$?
set -e
if [ "$STATUS" -eq 13 ]; then
  # A conditional-update loser may be a duplicate continuation rather than a
  # conflicting answer.  Re-read the gate before discarding the staged answer;
  # only the exact bead metadata proves that this event won the CAS.
  GATE_JSON=$(bd show "$GC_DECISION_BEAD_ID" --json)
  if printf '%s' "$GATE_JSON" | jq -e \
    --arg run "$GC_RUN_ID" \
    --arg decision "$GC_DECISION_ID" \
    --arg nonce "$GC_DECISION_NONCE" \
    --arg message "$(printf '%s' "$EVENT_JSON" | jq -r '.message_id')" \
    --arg event "$(printf '%s' "$EVENT_JSON" | jq -r '.event_id')" \
    --arg choice "$(printf '%s' "$EVENT_JSON" | jq -r '.choice')" '
      (if type == "array" then .[0] else . end) as $bead
      | (($bead.metadata // {}) as $metadata
        | ($metadata.decision_run_id // "") == $run
        and ($metadata.decision_id // "") == $decision
        and ($metadata.decision_nonce // "") == $nonce
        and ($metadata.decision_message_id // "") == $message
        and ($metadata.decision_event_id // "") == $event
        and ($metadata.decision_choice // "") == $choice
        and (($bead.status // "") == "in_progress"
          or ($bead.status // "") == "closed"))
    ' >/dev/null; then
    python3 "$GC_CONTRIBUTOR_ROOT/pack/scripts/discord-decision.py" ack \
      --socket "$GC_DISCORD_CHANNEL_SOCKET" \
      --run-id "$GC_RUN_ID" \
      --decision-id "$GC_DECISION_ID" \
      --event-id "$(printf '%s' "$EVENT_JSON" | jq -r '.event_id')" \
      --choice "$(printf '%s' "$EVENT_JSON" | jq -r '.choice')" \
      --accepted
    if [ "$(printf '%s' "$GATE_JSON" | jq -r '(if type == "array" then .[0] else . end).status // ""')" = "in_progress" ]; then
      set +e
      bd update "$GC_DECISION_BEAD_ID" \
        --if-assignee "$GC_DECISION_ASSIGNEE" \
        --if-status in_progress \
        --status closed
      CLOSE_STATUS=$?
      set -e
      if [ "$CLOSE_STATUS" -ne 0 ] && [ "$CLOSE_STATUS" -ne 13 ]; then
        exit "$CLOSE_STATUS"
      fi
      if [ "$CLOSE_STATUS" -eq 13 ] \
        && ! printf '%s' "$GATE_JSON" | jq -e \
          '(if type == "array" then .[0] else . end).status == "closed"' \
          >/dev/null; then
        exit 13
      fi
    fi
    python3 "$GC_CONTRIBUTOR_ROOT/pack/scripts/discord-decision.py" close \
      --socket "$GC_DISCORD_CHANNEL_SOCKET" \
      --run-id "$GC_RUN_ID" \
      --decision-id "$GC_DECISION_ID" \
      --event-id "$(printf '%s' "$EVENT_JSON" | jq -r '.event_id')" \
      --choice "$(printf '%s' "$EVENT_JSON" | jq -r '.choice')"
  else
    # Another answer won the authoritative bead, so this staged event must
    # never reopen or overwrite the gate.
    python3 "$GC_CONTRIBUTOR_ROOT/pack/scripts/discord-decision.py" reject \
      --socket "$GC_DISCORD_CHANNEL_SOCKET" \
      --run-id "$GC_RUN_ID" \
      --decision-id "$GC_DECISION_ID" \
      --event-id "$(printf '%s' "$EVENT_JSON" | jq -r '.event_id')" \
      --choice "$(printf '%s' "$EVENT_JSON" | jq -r '.choice')"
  fi
  exit 0
fi
test "$STATUS" -eq 0

python3 "$GC_CONTRIBUTOR_ROOT/pack/scripts/discord-decision.py" ack \
  --socket "$GC_DISCORD_CHANNEL_SOCKET" \
  --run-id "$GC_RUN_ID" \
  --decision-id "$GC_DECISION_ID" \
  --event-id "$(printf '%s' "$EVENT_JSON" | jq -r '.event_id')" \
  --choice "$(printf '%s' "$EVENT_JSON" | jq -r '.choice')" \
  --accepted

set +e
bd update "$GC_DECISION_BEAD_ID" \
  --if-assignee "$GC_DECISION_ASSIGNEE" \
  --if-status in_progress \
  --status closed
CLOSE_STATUS=$?
set -e
if [ "$CLOSE_STATUS" -ne 0 ] && [ "$CLOSE_STATUS" -ne 13 ]; then
  exit "$CLOSE_STATUS"
fi
if [ "$CLOSE_STATUS" -eq 13 ]; then
  GATE_JSON=$(bd show "$GC_DECISION_BEAD_ID" --json)
  printf '%s' "$GATE_JSON" | jq -e \
    --arg run "$GC_RUN_ID" \
    --arg decision "$GC_DECISION_ID" \
    --arg nonce "$GC_DECISION_NONCE" \
    --arg message "$(printf '%s' "$EVENT_JSON" | jq -r '.message_id')" \
    --arg event "$(printf '%s' "$EVENT_JSON" | jq -r '.event_id')" \
    --arg choice "$(printf '%s' "$EVENT_JSON" | jq -r '.choice')" '
      (if type == "array" then .[0] else . end) as $bead
      | (($bead.metadata // {}) as $metadata
        | ($bead.status // "") == "closed"
        and ($metadata.decision_run_id // "") == $run
        and ($metadata.decision_id // "") == $decision
        and ($metadata.decision_nonce // "") == $nonce
        and ($metadata.decision_message_id // "") == $message
        and ($metadata.decision_event_id // "") == $event
        and ($metadata.decision_choice // "") == $choice)
    ' >/dev/null
fi

python3 "$GC_CONTRIBUTOR_ROOT/pack/scripts/discord-decision.py" close \
  --socket "$GC_DISCORD_CHANNEL_SOCKET" \
  --run-id "$GC_RUN_ID" \
  --decision-id "$GC_DECISION_ID" \
  --event-id "$(printf '%s' "$EVENT_JSON" | jq -r '.event_id')" \
  --choice "$(printf '%s' "$EVENT_JSON" | jq -r '.choice')"
```

On restart, rerun this stage.  An `answered` prompt is returned with its
persisted event id and choice, allowing the blocked gate bead to close
without accepting a second response.  Never create a separate approval
record, signature, or evidence artifact.
