### Fixed

- The bootstrap-PSK and KK-session cryptoperiod defaults now share a single
  canonical definition in `d2b_contracts_zone_session`, re-exported by the
  child-local ZoneLink handler and the bus-side enrollment machine, so the two
  sides cannot silently drift apart on the values.