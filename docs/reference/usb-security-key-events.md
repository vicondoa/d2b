# USB security-key desktop observer events

The desktop observer receives security-key presentation events only through an
authenticated ComponentSession. The exact session contract is:

| Field | Value |
| --- | --- |
| endpoint purpose | `desktop-observer` |
| endpoint role | `desktop-observer` |
| service package | `d2b.notify.v2` |
| service | `NotifyService` |
| observer methods | `Subscribe`, `Acknowledge` |

The composition layer supplies a pre-authorized local transport. The observer
does not discover socket paths, retry a second endpoint, read an event JSONL
file, or treat a presentation projection as a control channel. Authentication,
service selection, limits, and transport binding come from the negotiated
ComponentSession.

## Bounded event payload

Each queued event has a monotonic `sequence`, `observedAtUnixMs`, and one
`event`. A subscription page contains at most 32 events and 16 KiB. The
observer retains at most 64 events and 64 KiB. If retention pressure drops an
older event, the next page reports `gapBeforePage: true`; consumers must refresh
their projection rather than infer missing state.

The event object is one of:

| `kind` | Additional fields | Terminal | Desktop notification |
| --- | --- | --- | --- |
| `started` | optional `rpId` | no | yes |
| `touchNeeded` | none | no | yes |
| `busy` | `detail.holderVm`, bounded `detail.waitingVms` | no | yes |
| `queued` | `queuePosition` | no | no |
| `blocked` | closed `reason` | no | yes |
| `timedOut` | none | yes | yes |
| `failed` | bounded presentation `reason` | yes | yes |
| `canceled` | none | yes | yes |
| `completed` | none | yes | no |

Every event also carries bounded `sessionId` and `vmName` fields. The maximum
encoded event size is 4 KiB. Session IDs are opaque correlation values; they
are not credentials or callback tokens.

Example:

```json
{
  "sequence": 42,
  "observedAtUnixMs": 1784243985000,
  "event": {
    "kind": "touchNeeded",
    "sessionId": "session-42",
    "vmName": "personal"
  }
}
```

`Acknowledge` is monotonic and idempotent. It releases retained events through
the acknowledged sequence. Acknowledging a sequence that the observer has
never published fails closed.

## Notification projection

Desktop notification summaries and bodies are sanitized, bounded presentation
text. They carry no nonce, callback authority, endpoint, command, or host path.
Actions, when composed, invoke the authenticated `InvokeAction` service method;
notification payloads and files never authorize an action.

Successful completion and queue bookkeeping are silent. Other lifecycle states
produce a user-visible notification without exposing transport diagnostics or
raw provider output.

## State projection

The observer may materialize `sk-state.json` for presentation consumers. This
file is a bounded read model, not durable authority:

```json
{
  "schemaVersion": 1,
  "updatedAt": 1784243985,
  "active": [
    {
      "sessionId": "session-42",
      "vmName": "personal",
      "lastEventKind": "touchNeeded",
      "lastEventAt": 1784243985,
      "isTerminal": false
    }
  ],
  "recentTerminal": []
}
```

The projection is limited to 32 KiB, 16 active ceremonies, and eight recent
terminal ceremonies. Active entries older than five minutes are omitted by
readers. Unknown schema versions and projections that exceed count, text, or
byte limits are rejected.

Projection absence never triggers a daemon, socket, alternate-file, or legacy
protocol fallback. Operators must repair the ComponentSession service rather
than use the read model as an offline control path.

## Waybar read model

`d2b-sk-waybar-helper` requires exactly one explicit projection path:

```console
d2b-sk-waybar-helper "$XDG_RUNTIME_DIR/d2b/sk-state.json"
```

It reads at most 32 KiB and emits at most 4 KiB of Waybar JSON. Missing or
unreadable input exits `1`; malformed, oversized, or unsupported input exits
`2`; an omitted or extra path exits `64`. The helper never connects to an
endpoint and never searches a fallback location.

The Waybar output contains only `text`, `tooltip`, and a closed CSS class:
`d2b-sk-idle`, `d2b-sk-active`, `d2b-sk-touch`, or `d2b-sk-busy`.

## Observability

The observer exposes four closed, low-cardinality measures for the local
observability provider: accepted events, dropped events, queue depth, and
projection entry count. VM names, session IDs, relying-party IDs, endpoint
paths, and event payloads are never observability labels.
