### Changed

- The Zone plane's per-resource Volume anchor projection is driven by the manager's durable-change stream: a Volume or VolumeBinding row committed by any writer (provider effect, declared child row, controller child-mutation bridge, API apply, or bundle ingest) re-materializes the projection through one subscription, coalesced into a bounded per-row registration instead of a store-wide reload.

### Removed

- Retired the per-provider-family anchor-refresh hook and its adapters, the controller bridge's per-call registry reload, and the bundle ingest's trailing reload. A provider family no longer has to remember to refresh the plane's anchor projection, so a family cannot omit it.
