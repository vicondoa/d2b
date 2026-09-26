### Fixed

- The published `Command` schema now carries the admission its parse
  enforces, so a value that satisfies the schema always deserializes.
  `CommandExec` publishes the control-free absolute path pattern (every
  `char::is_control` code point: C0, DEL, and C1, not NUL only), and
  `CommandArgvSlot` publishes `minLength: 1` plus the slot pattern: a
  brace-free, control-free literal, or exactly one whole-slot
  `{placeholder}` naming a well-formed payload parameter.

### Changed

- `CommandArgvSlot::parse` refuses a control character in a literal
  slot, matching the control-free executable rule and the pattern the
  published schema carries.
