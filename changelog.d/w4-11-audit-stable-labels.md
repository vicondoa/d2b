### Fixed

- Clipboard audit wire labels (`event`, `size`) are now explicit stable `as_str()` values instead of being derived from `Debug` formatting, so the wire record no longer changes if a variant's `Debug` output changes.