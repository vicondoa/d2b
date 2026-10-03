### Added

- Network membership can now be admitted per consumer while the host fabric
  stays shared. A consumer's network binding request names one Network, one
  consumer, one stable slot, the ports it may receive, and whether it may
  originate outbound connections; two processes on one Network keep separate
  traffic policy while the bridges, routes, ownership markers, NetworkManager
  policy, and firewall projection are realized once per Network and execution
  target.
- Releasing one consumer no longer removes shared fabric another consumer is
  still using, and a guest or host child-support ceiling now only bounds what a
  child may request instead of granting anything itself.
- A membership request never carries a pre-rendered firewall ruleset: the
  firewall stays the Network's single ownership-scoped projection, and a
  request that needs a realization facet the provider does not declare is
  refused rather than approximated.

### Changed

- A foreign nftables entry, host interface, route, or NetworkManager
  configuration occupying a Network's trusted slot now refuses the membership
  and leaves the observed host bytes untouched, matching the existing
  foreign-state behavior for firewall ownership.
