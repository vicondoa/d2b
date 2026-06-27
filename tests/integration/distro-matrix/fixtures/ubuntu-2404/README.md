# Ubuntu 24.04 Tier-1 smoke fixtures (W19)

This directory holds the expected-shape fixtures the W19 Tier-1 smoke
harness asserts against.

- `expected-audit-ops.txt`: list of broker operations that must
  appear in `/var/lib/d2b/audit/broker-<UTC-date>.jsonl` after a
  full smoke run.
- `expected-installer-artifacts.txt`: list of file paths the W15
  `RunHostInstall` broker op must materialize.

The smoke harness itself lives at
`tests/integration/distro-matrix/ubuntu-2404-tier1.sh`. Run it manually on an
Ubuntu 24.04 LTS x86_64 host with `/dev/kvm` and root:

```text
sudo D2B_REPO=/path/to/d2b \
     tests/integration/distro-matrix/ubuntu-2404-tier1.sh
```

On non-Ubuntu hosts the harness runs in scaffold-only mode (sets
`D2B_UBUNTU_SCAFFOLD_ONLY=1` automatically) and exercises the
helpers without performing any live install / VM start / SSH probe.
