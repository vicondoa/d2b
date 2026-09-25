### Fixed

- `NftBatch` no longer derives `Deserialize`: its `&'static str` table fields pinned the generated impl to `'de: 'static`, so the derive could never deserialize runtime input and misled readers; batches are recovered via `NftBatch::parse` instead.