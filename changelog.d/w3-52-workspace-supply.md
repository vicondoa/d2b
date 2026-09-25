### Changed

- Dropped unused workspace dependencies from three provider crates: `serde` from d2b-provider-activation-nixos, `schemars` from d2b-provider-audio-pipewire, and `sha2` from d2b-provider-guest-azure-container-apps, since no source or test in those crates references them.