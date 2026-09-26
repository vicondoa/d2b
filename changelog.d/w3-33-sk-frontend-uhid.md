### Changed

- d2b-sk-frontend: extract UHID event parsing into a testable `parse_event`
  and cover the byte-exact dispatch (output size field at payload offset
  4096, GET_REPORT id, lifecycle mapping, short-header error) with table
  tests, so a regression in the parse offsets fails the suite.