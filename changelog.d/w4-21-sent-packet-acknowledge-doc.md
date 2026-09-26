### Fixed

- `SentPacket::acknowledge` now documents its drop-semantics contract: consuming a sent packet releases its credit reservations and retained attachment file descriptors.