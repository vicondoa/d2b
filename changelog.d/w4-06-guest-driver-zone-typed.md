### Fixed

- `GuestDriverArgs.zone` is now a typed `ZoneId` instead of a raw `String`:
  the guest driver no longer re-parses the zone at construction, so an
  invalid zone can no longer panic the driver factory after the plane
  already validated it.