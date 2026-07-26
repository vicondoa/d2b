### Fixed

- Heavy test guards and shared shell helpers now invoke Bash cleanup builtins
  explicitly, avoiding confusing failures when a developer exports a tool
  wrapper function.
- Runtime census regeneration now refuses to erase any previously pinned test
  or crate identifier unless the committed census records that removal
  explicitly first.
