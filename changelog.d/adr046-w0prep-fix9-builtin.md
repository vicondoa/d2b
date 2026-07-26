### Fixed

- Heavy test guards and shared shell helpers now invoke Bash cleanup builtins
  explicitly, avoiding confusing failures when a developer exports a tool
  wrapper function.
