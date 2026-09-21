### Fixed

- Made the transport-vsock service tests deterministic under load: the open-deadline and close-grace assertions now run on tokio's paused virtual clock instead of racing real time, so a slow or starved machine can no longer let a fake delay outlive the deadline it is meant to be bounded by.