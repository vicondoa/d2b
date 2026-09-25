### Fixed

- Activation-nixos verification-fence failures now name the failing case and expected error instead of reporting only a line number, so a six-case regression identifies which trust or digest fence broke.
- The credential-entra consumer guard test now exercises `authorizes_consumer` against the exact Provider reference and a different one, so a guard regression fails the test instead of passing on two unrelated parse results.
- The Azure VM error test now pins each stable error code exactly, so an accidental code change is caught instead of an always-true emptiness check.