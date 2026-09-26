### Fixed

- The broker protocol receive path now peeks the 4-byte frame length prefix with `MSG_PEEK` and allocates exactly the declared frame size plus the prefix, instead of a fixed 1 MiB buffer per received frame; the same size-exact allocation applies to SCM_RIGHTS frame receipt, and sockets without `MSG_PEEK` support keep the fixed allocation.