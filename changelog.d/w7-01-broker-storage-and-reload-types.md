### Changed

- Broker storage/sync contract refusals carry a closed reason type:
  every refusal reason is one variant of that type instead of a slug
  spelled at the raise site, so a new refusal cannot misspell what the
  operator and the audit record see. The rendered text is unchanged,
  including the canonicalize failure that still appends its host error
  detail.

- The NetworkManager unmanaged reload behavior is a parsed contract
  value rather than a string re-checked at each call site.
  `reloadBehavior` resolves to the closed set `atomic-reload`, `none`,
  and the empty no-host-contract sentinel when the host artifact and the
  broker kernel payload are read, so a hand-declared typo fails at
  resolution instead of reaching the apply and remove paths. The wire
  value and its empty sentinel round-trip unchanged.
