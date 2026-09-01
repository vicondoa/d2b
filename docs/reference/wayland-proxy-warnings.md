# Wayland Provider warning catalog

**Diataxis category:** reference.

Wayland mediation is a Provider projection for a Zone Guest. Warnings are
advisory diagnostics; the security boundary itself remains fail-closed.

## Warning classes

| Class | Meaning |
| --- | --- |
| `baseline-global-denied` | A required application Wayland global was denied. |
| `high-risk-global-allowed` | A screen-capture, virtual-input, shell, or session-control global was explicitly allowed. |
| `unclassified-global-allowed` | An unreviewed protocol was explicitly allowed. |
| `clipboard-boundary-bypassed` | A request tried to bypass the d2b clipboard Provider. |
| `decoration-draw-failed` | Optional Provider-owned presentation metadata could not be rendered. |

Warnings must not include window titles, buffer contents, credentials, socket
paths, host paths, or private runtime identifiers. Guest application buffers
continue through the approved Provider path when an optional decoration fails.

## Inspection

```bash
d2b display list --zone work
d2b guest status work-app --zone work
d2b host doctor --read-only
```

Unknown or unsafe Wayland globals remain denied until a reviewed Provider
contract classifies them. Do not add a compositor socket or raw global allowlist
to a public Guest Resource.
