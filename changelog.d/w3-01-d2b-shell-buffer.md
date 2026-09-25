### Changed

- The CLI reuses one receive buffer per daemon connection instead of allocating and zeroing a fresh 1 MiB buffer for every frame, cutting per-poll allocation churn on the interactive shell path.
- `exec wait` now has tests pinning the guest exit-code passthrough and its out-of-range fallback, and `exec run --env` now has tests pinning the KEY=VALUE key validation, so regressions in either contract fail loudly instead of silently.