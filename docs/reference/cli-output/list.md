# `d2b list` output

`d2b list <RESOURCE_TYPE> --json` emits the v3 Resource API `ListResponse`
for one ResourceType in one Zone. The response is stable JSON with these
top-level fields:

| Field | Type | Semantics |
| --- | --- | --- |
| `resources` | array | Canonical resource envelopes for the requested type. |
| `snapshotRevision` | integer | Zone revision that anchors this page. |
| `nextCursor` | object or `null` | Opaque continuation cursor. |
| `truncated` | boolean | Whether another page is available. |
| `error` | object or `null` | Typed, bounded Resource API failure. |

The required positional ResourceType prevents an unbounded global inventory
scan. Use `--zone`, `--label-selector`, `--limit`, and `--page-token` to scope
the request. Human formatting is not a wire contract.

The pre-v3 untyped VM inventory form, `d2b list --json`, is retired. VM and
Guest inventory is queried through the corresponding v3 ResourceType instead.
