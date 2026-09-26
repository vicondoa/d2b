### Fixed

- The Wayland proxy's clipboard bridge decodes the refresh frame `d2b-clipd`
  writes instead of searching the raw byte stream for the literal
  `"type":"refresh_selection"` substring. A reformatted frame (extra
  whitespace or a different key order) and an unknown `type` tag used to
  disable clipboard refresh with no diagnostic; the bridge now deserializes
  each newline-delimited frame into the typed inbound frame shape, matches the
  `RefreshSelection` variant, and reports a frame it cannot decode through the
  rate-limited diagnostics as `reason=frame-decode-failed` while continuing to
  serve the refresh frames around it.
