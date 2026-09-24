# U42 d2b-provider-guest-qemu-media

## Verdict
Lean. Ship.

No in-crate deletions. The crate is the post-consolidation single home of the
QMP protocol vocabulary (`src/qmp/mod.rs`: QmpCommand, QmpGreeting, QmpSession,
QmpTimeout, QmpTransport, ScriptedQmpTransport, QmpVmStatus), the host-rule
admission vocabulary (DeviceAdmission/DeviceAdmissionError, AttachmentSlot),
and the adoption surface (AdoptionOutcome, ProcessIdentity, verify_identity) -
all consumed by the controller/state-machines within this crate and the few
outside readers (MEDIA_CONTRACT_ID, QEMU_MEDIA_IMPLEMENTATION_ID, PROVDR, etc).

## Byte-unique declaration homes (single-decl check)
The workspace-wide loop found exactly one `pub enum`/`pub struct`/`pub trait`
declaration of each of: QmpCommand, QmpGreeting, QmpSession, QmpTimeout,
QmpTransport, ScriptedQmpTransport, QmpVmStatus, AdoptionOutcome,
ProcessIdentity, DeviceAdmission, DeviceAdmissionError, AttachmentSlot,
AttachmentKind, NetworkAttachment, DeviceAttachment, Cardinality (imported from
d2b_resource_types::provider - toolkit re-export), IsolationPosture (imported
from d2b_resource_types::provider). Nothing re-declared here that has an
authority home elsewhere. KernelCaller is imported from d2b_resource_types::
operation (the U10 family seam) - no second home.

## Caller census (exclude this crate's own src; out-of-crate readers only)
- `WellKnownType::PROCESS/GUEST/VOLUME/...` - projected from
  d2b_resource_types::WellKnownType::ALL const table, 74-keeper feed
- MEDIA_CONTRACT_ID: 1 external reader (d2bd composition adjacency)
- QEMU_MEDIA_IMPLEMENTATION_ID: 15
- FINALIZER: 5
- PROVIDER_REF: 83

Zero-reader: none. Every item in this crate's public surface is either
multiplied across the workspace or is the single declaration home that
external crates import from.

## Consistency notes
- **Wire-shape/naming divergence (to U97):** `IsolationPosture` as a name
  appears in four crates with distinct variant vocabularies - canonical home
  is `d2b_resource_types::provider` (`Standard`/`UnsafeLocal`); divergences:
  workload.rs (`VirtualMachine`/`ProviderManaged`/`UnsafeLocal`),
  workload contracts v3/host.rs (`NoIsolation`), shell-terminal host_rules.rs
  (`Isolated`/`None`). This crate imports the canonical copy. Feed under U97.
- **Opacity seam note:** QmpCommand/QmpReply/QmpGreeting etc. here carry the
  redacted/abstracted QMP vocabulary this crate owns; this is the committed
  home for the module. No drift.
- **WellKnownType::to_resource_type_name::Vehicle names:** spot-checked
  round-trips to `activation-nixos.d2bus.org.NixosGeneration` etc. already
  consolidated at d2b-resource-runtime - no re-decl.
