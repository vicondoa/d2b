### Added

- The Endpoint owner derives the `EndpointBinding` rows its committed
  `Endpoint` row implies (`d2b-provider-endpoint`). `canonical_binding_rows`
  and `canonical_binding_row` take the Zone, the endpoint's own `EndpointSpec`,
  the endpoint reference, and the deliveries that row declares, and commit
  exactly one canonical `EndpointBindingSpec` row per delivery; a source row
  that declares no delivery derives no row. Each row carries the source's own
  `BindingSourceDecision` - the right the attachment kind requests, shared
  arbitration, and the realized facets taken from the kind's own
  `required_facets()` rather than a fixed pair, so a connect or a listen
  commits the endpoint descriptor alone while an attach commits the descriptor
  and the private presentation. The endpoint reference and the bounded purpose
  come from the committed endpoint row, so a delivery cannot reach a different
  endpoint or invent a purpose the endpoint never published, and the request
  the row was derived from travels beside the row bytes so the admission and
  the committed row cannot drift apart.
- A delivery is refused rather than committed unless the endpoint's own
  declaration admits it: the consumer must be one `BindingKind::Endpoint`
  admits (a `Host` never is, so a `Process` or an `EphemeralProcess` helper is
  derivable and a host-side need stays an admitted realization leg), the
  endpoint's subject allowlist must name it, the endpoint must declare the
  operation its attachment kind performs, an `attach` needs attachment
  capacity, and every facet the delivery rides on must be one the family
  declares it can realize (`ensure_realizable`, now the single place both the
  derivation and `EndpointBindingRegistry::admit` answer that).
- `binding_row_name` mints the deterministic row name from the KTD3 slot
  address - the bound endpoint, the consumer, and the consumer's own stable
  slot - never from a declaration position, so one relationship keeps one
  identity across restarts, reordering declarations never churns a name, and
  two deliveries claiming one consumer slot are refused as the one
  relationship declared twice.
