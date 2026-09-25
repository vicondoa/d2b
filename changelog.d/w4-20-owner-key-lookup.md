### Fixed

- Resource API mutation resolution no longer lists the entire Zone row set
  to resolve an owner: `owner_key_for` now asks the resource manager for the
  owner uid's key through the manager's uid index, so Delete and owner-less
  updates resolve ownership in constant time instead of scanning every row.