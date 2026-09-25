### Fixed

- `d2b-host-activation-helper` now documents the safety invariant of every
  production `unsafe` block (libc calls and the `errno` clear): `CString`
  NUL-termination, checked return values before use, and fd ownership /
  close-once discipline. No behavior change.