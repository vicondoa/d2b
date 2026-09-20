### Changed

- Broker operation rows a provider crate serves are now declared in that
  crate's `operations.json` and generated into the committed
  `docs/reference/policy/broker-operations.json` and its derived views, so
  adding or renaming a family operation needs no edit outside the declaring
  crate. The network and process families' 24 declared rows moved onto the
  new declaration surface; the drift and parity gates pin the generated
  artifacts and the declaration-to-descriptor agreement.