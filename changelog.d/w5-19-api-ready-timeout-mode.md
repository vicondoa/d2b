### Fixed

- `DaemonEvent::ApiReadyTimeout.mode` is now a closed two-variant `ApiReadyMode` enum (`Strict` / `NoWaitApi`) with kebab-case serde, so an invalid mode string is rejected on deserialize instead of landing in the preserved audit record; the daemon-events JSONL shape is unchanged (`"strict"` / `"no-wait-api"`).