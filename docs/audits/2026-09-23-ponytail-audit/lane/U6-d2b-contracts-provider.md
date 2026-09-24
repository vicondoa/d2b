# U6 d2b-contracts-provider

net: -0 lines, -0 deps

Contracts/types-layer crate. No production-code cut verified this pass.

- `yagni` validate_data_point_without_label_key_validation - public validator whose only distinguish
ing behavior (skip label-key validation) is bypassed by collapsing the flag at the sole external call
site (odometry-policy ingress_policy.rs:413 calls it with the flag used only to satisfy a ke-linearity
loop that can never fire given the frames it admits). Candidate, not verified: the collapse needs
d2b-provider-observability-otel's label-insertion model (owned by a different family), so it is
not an in-crate deletion.

## Consistency notes - d2b-contracts-provider is a contracts/types-layer crate (types-family U2-U11)

Flags the wire-shape skew the prior record's rejected macro consolidation (#C4: 76 Wire
deserialize blocks, identity macros, facade copies - still present, not re-flagged) will not
resolve. Credential Controller wire DTOs (`CredentialLeaseStatus`, `CredentialControllerCall`,
`CredentialProviderKind` accessors) are the canonical shape; the proto at
`proto/credential.proto` and the `CredentialStatus`/`CredentialAuditRecord` renderers (v3
credential / credential_controller) are the projection. No rebound drift found: the proto's
`camelCase` wire names match the serde `camelCase`/`kebabCase` rename sets, and the
credential_ref -> ResourceUid->digest redaction chain is uniform.

## Reopened refusals
- none

## Checked
- read-in-full: telemetry_policy.rs (998), telemetry_frame.rs (567), semantic-services
  audio/usb/security_key/telemetry/child_resources (226-942 each), credential.rs (1176),
  credential_controller.rs (1925), credential/service.rs (1461), provider.rs (4363),
  provider_registry.rs (283), v3/mod.rs
- verified caller-claims workspace-wide: grep -rn "<fn>" packages/*/src packages/*/tests
  packages/*/BUILD.bazel packages/*/integration nixos-modules/ docs/reference/policy/;
  no public DTO accessor orphaned, every dead label/value policy constant is projected
  onto an OTEL descriptor or validator allowlist; the frozentelemetry catalog matches its
  generated wire shape. CredentialControllerError variants CredentialControllerError::
  InvalidInput/OperationDenied/DeadlineExceeded/AlreadyRunning all have live constructors.
  Ledger item #C4 honored (not re-flagged on no new macro/record evidence). No dead public
  type or zero-caller facade found in the reviewed contracts crate. Lean already - a big
  contracts crate that is almost exclusively leaves hands on the wire.
