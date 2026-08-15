# Cloud Hypervisor Provider integration

Hermetic lifecycle tests use injected process and guest-control effect ports:

```text
cargo test -p d2b-provider-runtime-cloud-hypervisor
```

Host/KVM acceptance is a separate manual `make test-host-integration` lane.
It must prove real broker-spawned process ownership, authenticated
guest-control readiness, and restart adoption without a duplicate VMM.
