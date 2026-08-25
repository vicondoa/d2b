# Cloud Hypervisor Provider integration

Hermetic lifecycle tests use injected process and ComponentSession effect ports:

```text
bazel test //packages/d2b-provider-runtime-cloud-hypervisor:all
```

Host/KVM acceptance is a separate manual `make test-host-integration` lane.
It must prove real broker-spawned process ownership, authenticated
ComponentSession readiness, and restart adoption without a duplicate VMM.
