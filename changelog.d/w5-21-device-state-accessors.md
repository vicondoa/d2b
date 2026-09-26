### Fixed

- DeviceResourceState now keeps its three provider caches (TPM controllers, GPU controllers, GPU authority leases) private and exposes read-only typed accessor methods; the daemon's shared provider effects read the caches through the accessors, and the GPU authority-lease construction contract stays behind the driver crate.