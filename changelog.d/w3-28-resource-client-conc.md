### Changed

- Resource Watch streams track their open/closing/closed lifecycle with a
  single atomic state instead of two, matching the process-attach stream
  wrapper and making the rollback after a failed close explicit.