use crate::host::IfNameError;
use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Metadata, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, path::PathBuf, str::FromStr};

pub const SUCCESS_EXIT_CODE: u8 = 0;
pub const GENERIC_CLI_EXIT_CODE: u8 = 1;
pub const USAGE_EXIT_CODE: u8 = 2;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, PartialOrd, Ord,
)]
pub enum Kind {
    #[serde(rename = "authz-not-a-launcher")]
    AuthzNotALauncher,
    #[serde(rename = "authz-audit-requires-admin")]
    AuthzAuditRequiresAdmin,
    #[serde(rename = "wire-version-mismatch")]
    WireVersionMismatch,
    #[serde(rename = "wire-frame-too-large")]
    WireFrameTooLarge,
    #[serde(rename = "wire-unknown-field")]
    WireUnknownField,
    #[serde(rename = "wire-ifname-invalid")]
    WireIfNameInvalid,
    #[serde(rename = "wire-malformed-json")]
    WireMalformedJson,
    #[serde(rename = "guest-shell-disabled")]
    GuestShellDisabled,
    #[serde(rename = "broker-unimplemented")]
    BrokerUnimplemented,
    #[serde(rename = "broker-validation-failed")]
    BrokerValidationFailed,
    #[serde(rename = "manifest-parse-error")]
    ManifestParseError,
    /// Bundle's emitted manifestVersion is not the version the running
    /// daemon supports. Stale bundles MUST be rejected with this distinct
    /// kind so operators can correlate the failure to a re-render of the
    /// bundle, not to a parse-side regression.
    #[serde(rename = "manifest-version-mismatch")]
    ManifestVersionMismatch,
    #[serde(rename = "internal-io")]
    InternalIo,
    /// Bundle artifact failed tamper-resistance check (symlink, owner,
    /// mode, or SHA-256 hash mismatch).
    #[serde(rename = "bundle-tampered")]
    BundleTampered,
    /// A provider required by an audio or console operation is present but
    /// not in a state where enforcement can proceed (e.g. expected guestd
    /// agent absent). Operator remediation required.
    #[serde(rename = "provider-misconfigured")]
    ProviderMisconfigured,
    #[serde(rename = "provider-artifact-output-ambiguous")]
    ProviderArtifactOutputAmbiguous,
    #[serde(rename = "provider-artifact-output-shape-unknown")]
    ProviderArtifactOutputShapeUnknown,
    #[serde(rename = "provider-required-output-absent")]
    ProviderRequiredOutputAbsent,
    #[serde(rename = "provider-required-output-not-regular")]
    ProviderRequiredOutputNotRegular,
    #[serde(rename = "provider-layout-entry-unexpected")]
    ProviderLayoutEntryUnexpected,
    #[serde(rename = "provider-executable-not-elf")]
    ProviderExecutableNotElf,
    #[serde(rename = "provider-launch-format-rejected")]
    ProviderLaunchFormatRejected,
    #[serde(rename = "provider-launch-permission-denied")]
    ProviderLaunchPermissionDenied,
    #[serde(rename = "provider-component-execution-invalid")]
    ProviderComponentExecutionInvalid,
    #[serde(rename = "provider-executable-declaration-inconsistent")]
    ProviderExecutableDeclarationInconsistent,
    #[serde(rename = "provider-executable-not-executable")]
    ProviderExecutableNotExecutable,
    #[serde(rename = "provider-signature-publisher-unregistered")]
    ProviderSignaturePublisherUnregistered,
    #[serde(rename = "provider-signature-id-unresolvable")]
    ProviderSignatureIdUnresolvable,
    #[serde(rename = "provider-signature-malformed")]
    ProviderSignatureMalformed,
    #[serde(rename = "provider-signature-verification-failed")]
    ProviderSignatureVerificationFailed,
    #[serde(rename = "provider-digest-mismatch")]
    ProviderDigestMismatch,
    #[serde(rename = "provider-manifest-not-canonical")]
    ProviderManifestNotCanonical,
    #[serde(rename = "provider-executable-name-invalid")]
    ProviderExecutableNameInvalid,
    #[serde(rename = "provider-executable-set-empty")]
    ProviderExecutableSetEmpty,
    #[serde(rename = "provider-executable-not-regular")]
    ProviderExecutableNotRegular,
    #[serde(rename = "provider-executable-set-mismatch")]
    ProviderExecutableSetMismatch,
    #[serde(rename = "provider-binary-ref-unresolved")]
    ProviderBinaryRefUnresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ErrorKindRecord {
    pub kind: Kind,
    pub exit_code: u8,
    pub owning_command: &'static str,
    pub message_template: &'static str,
    pub remediation: &'static str,
    pub docs_anchor: &'static str,
}

static ERROR_KIND_RECORDS: [ErrorKindRecord; 37] = [
    ErrorKindRecord {
        kind: Kind::AuthzNotALauncher,
        exit_code: 10,
        owning_command: "daemon-api/Hello",
        message_template: "peer uid {peer_uid} is not authorized to use the launcher API",
        remediation: "Add the caller to d2b.site.launcherUsers or retry with an authorized launcher account.",
        docs_anchor: "#authz-not-a-launcher",
    },
    ErrorKindRecord {
        kind: Kind::AuthzAuditRequiresAdmin,
        exit_code: 11,
        owning_command: "audit",
        message_template: "peer uid {peer_uid} is not authorized to read broker audit records",
        remediation: "Add the caller to d2b.site.adminUsers before retrying the audit read path.",
        docs_anchor: "#authz-audit-requires-admin",
    },
    ErrorKindRecord {
        kind: Kind::WireVersionMismatch,
        exit_code: 20,
        owning_command: "daemon-api/Hello",
        message_template: "client version requirement {client} does not include server version {server}",
        remediation: "Upgrade or downgrade the client so its declared semver range includes the server version.",
        docs_anchor: "#wire-version-mismatch",
    },
    ErrorKindRecord {
        kind: Kind::WireFrameTooLarge,
        exit_code: 21,
        owning_command: "daemon-api/frame",
        message_template: "received frame of {received} bytes exceeds the {limit}-byte limit",
        remediation: "Reduce the request body so the framed JSON payload stays at or below 1 MiB.",
        docs_anchor: "#wire-frame-too-large",
    },
    ErrorKindRecord {
        kind: Kind::WireUnknownField,
        exit_code: 22,
        owning_command: "daemon-api/request",
        message_template: "{type_name} rejected unknown field `{field}`",
        remediation: "Remove fields that are not documented for this request or response type.",
        docs_anchor: "#wire-unknown-field",
    },
    ErrorKindRecord {
        kind: Kind::WireIfNameInvalid,
        exit_code: 23,
        owning_command: "daemon-api/request",
        message_template: "wire interface name is invalid: {reason}",
        remediation: "Use a Linux interface name that fits in IFNAMSIZ-1 and only contains [A-Za-z0-9_-].",
        docs_anchor: "#wire-ifname-invalid",
    },
    ErrorKindRecord {
        kind: Kind::WireMalformedJson,
        exit_code: 24,
        owning_command: "daemon-api/request",
        message_template: "{type_name} could not be decoded from JSON (opaque reason: {opaque_reason})",
        remediation: "Send a complete JSON object that matches the documented wire schema.",
        docs_anchor: "#wire-malformed-json",
    },
    ErrorKindRecord {
        kind: Kind::GuestShellDisabled,
        exit_code: 70,
        owning_command: "shell",
        message_template: "persistent guest shell is not available for this VM",
        remediation: "Enable d2b.vms.<vm>.guest.shell when the shell runtime is available, rebuild the guest, and retry.",
        docs_anchor: "#guest-shell-disabled",
    },
    ErrorKindRecord {
        kind: Kind::BrokerUnimplemented,
        exit_code: 30,
        owning_command: "daemon-api/broker",
        message_template: "broker operation {operation} is not implemented in this build",
        remediation: "Upgrade to a build that implements this operation.",
        docs_anchor: "#broker-unimplemented",
    },
    ErrorKindRecord {
        kind: Kind::BrokerValidationFailed,
        exit_code: 31,
        owning_command: "daemon-api/broker",
        message_template: "broker validation failed for opaque target {what}",
        remediation: "Re-render the trusted bundle artifacts and retry with the newly emitted opaque identifiers.",
        docs_anchor: "#broker-validation-failed",
    },
    ErrorKindRecord {
        kind: Kind::ManifestParseError,
        exit_code: 40,
        owning_command: "status",
        message_template: "could not parse manifest artifact {artifact} (opaque reason: {opaque_reason})",
        remediation: "Re-render the manifest bundle and retry with the committed schema-compatible artifact set.",
        docs_anchor: "#manifest-parse-error",
    },
    ErrorKindRecord {
        kind: Kind::ManifestVersionMismatch,
        exit_code: 41,
        owning_command: "status",
        message_template: "manifest {artifact} declared an incompatible manifestVersion (opaque reason: {opaque_reason})",
        remediation: "Re-run `nixos-rebuild switch` against an updated d2b input pinning the daemon's supported manifestVersion. Manifest version changes do not ship a compatibility window.",
        docs_anchor: "#manifest-version-mismatch",
    },
    ErrorKindRecord {
        kind: Kind::InternalIo,
        exit_code: 50,
        owning_command: "daemon-api/internal",
        message_template: "an internal I/O step failed (opaque reason: {opaque_reason})",
        remediation: "Retry the command; if the error persists, inspect the daemon logs with the opaque reason token.",
        docs_anchor: "#internal-io",
    },
    ErrorKindRecord {
        kind: Kind::BundleTampered,
        exit_code: 60,
        owning_command: "bundle-load",
        message_template: "bundle artifact {path} failed tamper-resistance check: {reason}",
        remediation: "Re-run `nixos-rebuild switch` to restore the bundle artifacts to their signed state.",
        docs_anchor: "#bundle-tampered",
    },
    ErrorKindRecord {
        kind: Kind::ProviderMisconfigured,
        exit_code: 80,
        owning_command: "provider",
        message_template: "provider for {vm} is misconfigured: {reason}",
        remediation: "Check the provider configuration for the VM and verify the expected guestd-compatible agent or sidecar is running.",
        docs_anchor: "#provider-misconfigured",
    },
    ErrorKindRecord {
        kind: Kind::ProviderArtifactOutputAmbiguous,
        exit_code: 81,
        owning_command: "provider-catalog",
        message_template: "provider artifact {artifact} pins an ambiguous Nix output ({outputs})",
        remediation: "Select the Provider-carrying output explicitly; a non-first raw derivation output is accepted, while a raw derivation's first output must be repackaged.",
        docs_anchor: "#provider-artifact-output-ambiguous",
    },
    ErrorKindRecord {
        kind: Kind::ProviderArtifactOutputShapeUnknown,
        exit_code: 82,
        owning_command: "provider-catalog",
        message_template: "provider artifact {artifact} has an unrecognized Nix output shape",
        remediation: "Supply a derivation or a store path rather than a hand-built package attribute set.",
        docs_anchor: "#provider-artifact-output-shape-unknown",
    },
    ErrorKindRecord {
        kind: Kind::ProviderRequiredOutputAbsent,
        exit_code: 83,
        owning_command: "resource-compiler",
        message_template: "provider artifact {artifact} is missing required output {layout}",
        remediation: "Install the required Provider artifact file at its fixed layout-relative path and rebuild.",
        docs_anchor: "#provider-required-output-absent",
    },
    ErrorKindRecord {
        kind: Kind::ProviderRequiredOutputNotRegular,
        exit_code: 84,
        owning_command: "resource-compiler",
        message_template: "provider artifact {artifact} output {layout} is not a regular file ({file_type})",
        remediation: "Materialize the required Provider artifact output as a regular file and rebuild.",
        docs_anchor: "#provider-required-output-not-regular",
    },
    ErrorKindRecord {
        kind: Kind::ProviderLayoutEntryUnexpected,
        exit_code: 85,
        owning_command: "resource-compiler",
        message_template: "provider artifact {artifact} has unexpected layout entry {entry}",
        remediation: "Remove unpinned entries from share/d2b/provider and rebuild the Provider artifact.",
        docs_anchor: "#provider-layout-entry-unexpected",
    },
    ErrorKindRecord {
        kind: Kind::ProviderExecutableNotElf,
        exit_code: 86,
        owning_command: "resource-compiler",
        message_template: "provider artifact {artifact} executable {entry} is not a supported ELF image ({magic})",
        remediation: "Package interpreted entry points with d2b.lib.buildProviderElfShim and rebuild.",
        docs_anchor: "#provider-executable-not-elf",
    },
    ErrorKindRecord {
        kind: Kind::ProviderLaunchFormatRejected,
        exit_code: 87,
        owning_command: "provider-launcher",
        message_template: "provider artifact {artifact} component {component} binary {binary_ref} was rejected by the kernel ({errno})",
        remediation: "Install a valid executable image for the component and rebuild the Provider artifact.",
        docs_anchor: "#provider-launch-format-rejected",
    },
    ErrorKindRecord {
        kind: Kind::ProviderLaunchPermissionDenied,
        exit_code: 88,
        owning_command: "provider-launcher",
        message_template: "provider artifact {artifact} component {component} binary {binary_ref} could not be executed ({errno})",
        remediation: "Set the execute bit in the Provider derivation install step and rebuild.",
        docs_anchor: "#provider-launch-permission-denied",
    },
    ErrorKindRecord {
        kind: Kind::ProviderComponentExecutionInvalid,
        exit_code: 89,
        owning_command: "resource-compiler",
        message_template: "provider artifact {artifact} component {component} has an invalid execution declaration",
        remediation: "Name the component binary or remove the component when it is not launchable.",
        docs_anchor: "#provider-component-execution-invalid",
    },
    ErrorKindRecord {
        kind: Kind::ProviderExecutableDeclarationInconsistent,
        exit_code: 90,
        owning_command: "resource-compiler",
        message_template: "provider artifact {artifact} has an inconsistent executable declaration ({side})",
        remediation: "Make package.executableDigests agree with the Provider component execution declarations.",
        docs_anchor: "#provider-executable-declaration-inconsistent",
    },
    ErrorKindRecord {
        kind: Kind::ProviderExecutableNotExecutable,
        exit_code: 91,
        owning_command: "resource-compiler",
        message_template: "provider artifact {artifact} executable {entry} has no execute bit ({mode})",
        remediation: "Set the execute bit in the Provider derivation install step and rebuild.",
        docs_anchor: "#provider-executable-not-executable",
    },
    ErrorKindRecord {
        kind: Kind::ProviderSignaturePublisherUnregistered,
        exit_code: 92,
        owning_command: "resource-compiler",
        message_template: "provider artifact {artifact} publisher {publisher} is not registered",
        remediation: "Register the public publisher key at d2b.zones.<zone>.trustedPublishers.<publisher> or use a registered publisher.",
        docs_anchor: "#provider-signature-publisher-unregistered",
    },
    ErrorKindRecord {
        kind: Kind::ProviderSignatureIdUnresolvable,
        exit_code: 93,
        owning_command: "resource-compiler",
        message_template: "provider artifact {artifact} signature {signature_id} cannot be resolved for publisher {publisher}",
        remediation: "Republish with a registered signature or correct the catalog entry.",
        docs_anchor: "#provider-signature-id-unresolvable",
    },
    ErrorKindRecord {
        kind: Kind::ProviderSignatureMalformed,
        exit_code: 94,
        owning_command: "resource-compiler",
        message_template: "provider artifact {artifact} signature has {observed} bytes; expected 64",
        remediation: "Fix the detached signature file so it contains exactly 64 octets and rebuild.",
        docs_anchor: "#provider-signature-malformed",
    },
    ErrorKindRecord {
        kind: Kind::ProviderSignatureVerificationFailed,
        exit_code: 95,
        owning_command: "resource-compiler",
        message_template: "provider artifact {artifact} signature verification failed for {signature_id}",
        remediation: "Re-sign the canonical Provider manifest with the registered publisher key and rebuild.",
        docs_anchor: "#provider-signature-verification-failed",
    },
    ErrorKindRecord {
        kind: Kind::ProviderDigestMismatch,
        exit_code: 96,
        owning_command: "resource-compiler",
        message_template: "provider artifact {artifact} digest {digest_name} differs between {source_a} and {source_b}",
        remediation: "Regenerate the canonical Provider artifact and update the corresponding trusted digest before rebuilding.",
        docs_anchor: "#provider-digest-mismatch",
    },
    ErrorKindRecord {
        kind: Kind::ProviderManifestNotCanonical,
        exit_code: 97,
        owning_command: "resource-compiler",
        message_template: "provider artifact {artifact} output {layout} is not canonical at byte {offset}",
        remediation: "Run d2b-provider-toolkit manifest emit --out <path>, then d2b-provider-toolkit manifest verify <path>, and rebuild.",
        docs_anchor: "#provider-manifest-not-canonical",
    },
    ErrorKindRecord {
        kind: Kind::ProviderExecutableNameInvalid,
        exit_code: 98,
        owning_command: "resource-compiler",
        message_template: "provider artifact {artifact} executable name {entry} is invalid",
        remediation: "Use a UTF-8 executable name matching ^[a-z][a-z0-9-]*$ and rebuild.",
        docs_anchor: "#provider-executable-name-invalid",
    },
    ErrorKindRecord {
        kind: Kind::ProviderExecutableSetEmpty,
        exit_code: 99,
        owning_command: "resource-compiler",
        message_template: "provider artifact {artifact} has an empty executable set",
        remediation: "Remove bin/ and declare an empty executable set, or install a launchable ELF and declare its digest.",
        docs_anchor: "#provider-executable-set-empty",
    },
    ErrorKindRecord {
        kind: Kind::ProviderExecutableNotRegular,
        exit_code: 12,
        owning_command: "resource-compiler",
        message_template: "provider artifact {artifact} executable {entry} is not a regular file ({file_type})",
        remediation: "Materialize each Provider executable as a regular file and rebuild.",
        docs_anchor: "#provider-executable-not-regular",
    },
    ErrorKindRecord {
        kind: Kind::ProviderExecutableSetMismatch,
        exit_code: 13,
        owning_command: "resource-compiler",
        message_template: "provider artifact {artifact} executable set differs ({count} entries)",
        remediation: "Make the bin/ entry set equal the signed package.executableDigests key set and rebuild.",
        docs_anchor: "#provider-executable-set-mismatch",
    },
    ErrorKindRecord {
        kind: Kind::ProviderBinaryRefUnresolved,
        exit_code: 14,
        owning_command: "resource-compiler",
        message_template: "provider artifact {artifact} component {component} references unknown binary {binary_ref}",
        remediation: "Set binaryRef to a key in package.executableDigests or add the matching executable to the artifact.",
        docs_anchor: "#provider-binary-ref-unresolved",
    },
];

impl Kind {
    pub const fn discriminant(self) -> &'static str {
        self.slug()
    }

    pub const fn slug(self) -> &'static str {
        match self {
            Self::AuthzNotALauncher => "authz-not-a-launcher",
            Self::AuthzAuditRequiresAdmin => "authz-audit-requires-admin",
            Self::WireVersionMismatch => "wire-version-mismatch",
            Self::WireFrameTooLarge => "wire-frame-too-large",
            Self::WireUnknownField => "wire-unknown-field",
            Self::WireIfNameInvalid => "wire-ifname-invalid",
            Self::WireMalformedJson => "wire-malformed-json",
            Self::GuestShellDisabled => "guest-shell-disabled",
            Self::BrokerUnimplemented => "broker-unimplemented",
            Self::BrokerValidationFailed => "broker-validation-failed",
            Self::ManifestParseError => "manifest-parse-error",
            Self::ManifestVersionMismatch => "manifest-version-mismatch",
            Self::InternalIo => "internal-io",
            Self::BundleTampered => "bundle-tampered",
            Self::ProviderMisconfigured => "provider-misconfigured",
            Self::ProviderArtifactOutputAmbiguous => "provider-artifact-output-ambiguous",
            Self::ProviderArtifactOutputShapeUnknown => "provider-artifact-output-shape-unknown",
            Self::ProviderRequiredOutputAbsent => "provider-required-output-absent",
            Self::ProviderRequiredOutputNotRegular => "provider-required-output-not-regular",
            Self::ProviderLayoutEntryUnexpected => "provider-layout-entry-unexpected",
            Self::ProviderExecutableNotElf => "provider-executable-not-elf",
            Self::ProviderLaunchFormatRejected => "provider-launch-format-rejected",
            Self::ProviderLaunchPermissionDenied => "provider-launch-permission-denied",
            Self::ProviderComponentExecutionInvalid => "provider-component-execution-invalid",
            Self::ProviderExecutableDeclarationInconsistent => {
                "provider-executable-declaration-inconsistent"
            }
            Self::ProviderExecutableNotExecutable => "provider-executable-not-executable",
            Self::ProviderSignaturePublisherUnregistered => {
                "provider-signature-publisher-unregistered"
            }
            Self::ProviderSignatureIdUnresolvable => "provider-signature-id-unresolvable",
            Self::ProviderSignatureMalformed => "provider-signature-malformed",
            Self::ProviderSignatureVerificationFailed => "provider-signature-verification-failed",
            Self::ProviderDigestMismatch => "provider-digest-mismatch",
            Self::ProviderManifestNotCanonical => "provider-manifest-not-canonical",
            Self::ProviderExecutableNameInvalid => "provider-executable-name-invalid",
            Self::ProviderExecutableSetEmpty => "provider-executable-set-empty",
            Self::ProviderExecutableNotRegular => "provider-executable-not-regular",
            Self::ProviderExecutableSetMismatch => "provider-executable-set-mismatch",
            Self::ProviderBinaryRefUnresolved => "provider-binary-ref-unresolved",
        }
    }

    pub const fn as_str(self) -> &'static str {
        self.slug()
    }

    pub const fn exit_code(self) -> u8 {
        self.record().exit_code
    }

    pub const fn owning_command(self) -> &'static str {
        self.record().owning_command
    }

    pub const fn message_template(self) -> &'static str {
        self.record().message_template
    }

    pub const fn remediation(self) -> &'static str {
        self.record().remediation
    }

    pub const fn docs_anchor(self) -> &'static str {
        self.record().docs_anchor
    }

    pub const fn record(self) -> &'static ErrorKindRecord {
        match self {
            Self::AuthzNotALauncher => &ERROR_KIND_RECORDS[0],
            Self::AuthzAuditRequiresAdmin => &ERROR_KIND_RECORDS[1],
            Self::WireVersionMismatch => &ERROR_KIND_RECORDS[2],
            Self::WireFrameTooLarge => &ERROR_KIND_RECORDS[3],
            Self::WireUnknownField => &ERROR_KIND_RECORDS[4],
            Self::WireIfNameInvalid => &ERROR_KIND_RECORDS[5],
            Self::WireMalformedJson => &ERROR_KIND_RECORDS[6],
            Self::GuestShellDisabled => &ERROR_KIND_RECORDS[7],
            Self::BrokerUnimplemented => &ERROR_KIND_RECORDS[8],
            Self::BrokerValidationFailed => &ERROR_KIND_RECORDS[9],
            Self::ManifestParseError => &ERROR_KIND_RECORDS[10],
            Self::ManifestVersionMismatch => &ERROR_KIND_RECORDS[11],
            Self::InternalIo => &ERROR_KIND_RECORDS[12],
            Self::BundleTampered => &ERROR_KIND_RECORDS[13],
            Self::ProviderMisconfigured => &ERROR_KIND_RECORDS[14],
            Self::ProviderArtifactOutputAmbiguous => &ERROR_KIND_RECORDS[15],
            Self::ProviderArtifactOutputShapeUnknown => &ERROR_KIND_RECORDS[16],
            Self::ProviderRequiredOutputAbsent => &ERROR_KIND_RECORDS[17],
            Self::ProviderRequiredOutputNotRegular => &ERROR_KIND_RECORDS[18],
            Self::ProviderLayoutEntryUnexpected => &ERROR_KIND_RECORDS[19],
            Self::ProviderExecutableNotElf => &ERROR_KIND_RECORDS[20],
            Self::ProviderLaunchFormatRejected => &ERROR_KIND_RECORDS[21],
            Self::ProviderLaunchPermissionDenied => &ERROR_KIND_RECORDS[22],
            Self::ProviderComponentExecutionInvalid => &ERROR_KIND_RECORDS[23],
            Self::ProviderExecutableDeclarationInconsistent => &ERROR_KIND_RECORDS[24],
            Self::ProviderExecutableNotExecutable => &ERROR_KIND_RECORDS[25],
            Self::ProviderSignaturePublisherUnregistered => &ERROR_KIND_RECORDS[26],
            Self::ProviderSignatureIdUnresolvable => &ERROR_KIND_RECORDS[27],
            Self::ProviderSignatureMalformed => &ERROR_KIND_RECORDS[28],
            Self::ProviderSignatureVerificationFailed => &ERROR_KIND_RECORDS[29],
            Self::ProviderDigestMismatch => &ERROR_KIND_RECORDS[30],
            Self::ProviderManifestNotCanonical => &ERROR_KIND_RECORDS[31],
            Self::ProviderExecutableNameInvalid => &ERROR_KIND_RECORDS[32],
            Self::ProviderExecutableSetEmpty => &ERROR_KIND_RECORDS[33],
            Self::ProviderExecutableNotRegular => &ERROR_KIND_RECORDS[34],
            Self::ProviderExecutableSetMismatch => &ERROR_KIND_RECORDS[35],
            Self::ProviderBinaryRefUnresolved => &ERROR_KIND_RECORDS[36],
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.discriminant())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Authz(AuthzError),
    Wire(WireError),
    Broker(BrokerError),
    Manifest(ManifestError),
    Internal(InternalError),
    Bundle(BundleError),
    Audio(AudioError),
    PublicEnvelope(ErrorEnvelope),
}

impl Error {
    pub fn not_a_launcher(peer_uid: u32) -> Self {
        Self::Authz(AuthzError::NotALauncher { peer_uid })
    }

    pub fn audit_requires_admin(peer_uid: u32) -> Self {
        Self::Authz(AuthzError::AuditRequiresAdmin { peer_uid })
    }

    pub fn version_mismatch(client: SemverRange, server: Version) -> Self {
        Self::Wire(WireError::VersionMismatch { client, server })
    }

    pub fn frame_too_large(received: u64, limit: u64) -> Self {
        Self::Wire(WireError::FrameTooLarge { received, limit })
    }

    pub fn unknown_field(type_name: &'static str, field: impl Into<String>) -> Self {
        Self::Wire(WireError::UnknownField {
            type_name,
            field: field.into(),
        })
    }

    pub fn if_name_invalid(reason: IfNameError) -> Self {
        Self::Wire(WireError::IfNameInvalid { reason })
    }

    pub fn malformed_json(type_name: &'static str, opaque_reason: impl Into<String>) -> Self {
        Self::Wire(WireError::MalformedJson {
            type_name,
            opaque_reason: opaque_reason.into(),
        })
    }

    pub fn guest_shell_disabled() -> Self {
        Self::Wire(WireError::GuestShellDisabled)
    }

    pub fn broker_unimplemented(operation: BrokerOp, target_wave: u8) -> Self {
        Self::Broker(BrokerError::Unimplemented {
            operation,
            target_wave,
        })
    }

    pub fn broker_validation_failed(what: impl Into<String>) -> Self {
        Self::Broker(BrokerError::ValidationFailed { what: what.into() })
    }

    pub fn manifest_parse_error(artifact: &'static str, opaque_reason: impl Into<String>) -> Self {
        Self::Manifest(ManifestError::ParseError {
            artifact,
            opaque_reason: opaque_reason.into(),
        })
    }

    /// Typed version-mismatch helper.
    /// Distinct from `manifest_parse_error` so operators see kind
    /// `manifest-version-mismatch` instead of `manifest-parse-error`
    /// when a bundle render is stale.
    pub fn manifest_version_mismatch(
        artifact: &'static str,
        opaque_reason: impl Into<String>,
    ) -> Self {
        Self::Manifest(ManifestError::VersionMismatch {
            artifact,
            opaque_reason: opaque_reason.into(),
        })
    }

    pub fn internal_io(opaque_reason: impl Into<String>) -> Self {
        Self::Internal(InternalError::Io {
            opaque_reason: opaque_reason.into(),
        })
    }

    pub fn bundle_tampered(path: PathBuf, reason: impl Into<String>) -> Self {
        Self::Bundle(BundleError::Tampered {
            path,
            reason: reason.into(),
        })
    }

    /// Provider required by an audio or console operation is present but
    /// misconfigured (e.g. expected guestd agent absent).
    pub fn provider_misconfigured(vm: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Audio(AudioError::ProviderMisconfigured {
            vm: vm.into(),
            reason: reason.into(),
        })
    }

    pub const fn all_kinds() -> &'static [ErrorKindRecord] {
        &ERROR_KIND_RECORDS
    }

    pub fn kind(&self) -> Kind {
        match self {
            Self::Authz(error) => error.kind(),
            Self::Wire(error) => error.kind(),
            Self::Broker(error) => error.kind(),
            Self::Manifest(error) => error.kind(),
            Self::Internal(error) => error.kind(),
            Self::Bundle(error) => error.kind(),
            Self::Audio(error) => error.kind(),
            Self::PublicEnvelope(envelope) => envelope.kind,
        }
    }

    pub fn code(&self) -> u8 {
        match self {
            Self::PublicEnvelope(envelope) => envelope.code,
            _ => self.kind().exit_code(),
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Authz(error) => error.message(),
            Self::Wire(error) => error.message(),
            Self::Broker(error) => error.message(),
            Self::Manifest(error) => error.message(),
            Self::Internal(error) => error.message(),
            Self::Bundle(error) => error.message(),
            Self::Audio(error) => error.message(),
            Self::PublicEnvelope(envelope) => envelope.message.clone(),
        }
    }

    pub fn message_template(&self) -> &'static str {
        self.kind().message_template()
    }

    pub fn remediation(&self) -> &str {
        match self {
            Self::PublicEnvelope(envelope) => envelope.remediation.as_str(),
            _ => self.kind().remediation(),
        }
    }

    pub fn docs_anchor(&self) -> String {
        match self {
            Self::PublicEnvelope(envelope) => envelope.docs_anchor.clone(),
            _ => format!("docs/reference/error-codes.md{}", self.kind().docs_anchor()),
        }
    }

    pub fn owning_command(&self) -> &str {
        match self {
            Self::PublicEnvelope(envelope) => envelope.owning_command.as_str(),
            _ => self.kind().owning_command(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for Error {}

impl From<AuthzError> for Error {
    fn from(value: AuthzError) -> Self {
        Self::Authz(value)
    }
}

impl From<WireError> for Error {
    fn from(value: WireError) -> Self {
        Self::Wire(value)
    }
}

impl From<BrokerError> for Error {
    fn from(value: BrokerError) -> Self {
        Self::Broker(value)
    }
}

impl From<ManifestError> for Error {
    fn from(value: ManifestError) -> Self {
        Self::Manifest(value)
    }
}

impl From<InternalError> for Error {
    fn from(value: InternalError) -> Self {
        Self::Internal(value)
    }
}

impl From<BundleError> for Error {
    fn from(value: BundleError) -> Self {
        Self::Bundle(value)
    }
}

impl From<AudioError> for Error {
    fn from(value: AudioError) -> Self {
        Self::Audio(value)
    }
}

/// Errors for audio provider operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioError {
    /// Provider required by an audio or console operation is present but
    /// misconfigured (e.g. expected guestd agent absent).
    ProviderMisconfigured { vm: String, reason: String },
}

impl AudioError {
    pub const fn kind(&self) -> Kind {
        match self {
            Self::ProviderMisconfigured { .. } => Kind::ProviderMisconfigured,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::ProviderMisconfigured { vm, reason } => {
                format!("provider for {vm} is misconfigured: {reason}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthzError {
    NotALauncher { peer_uid: u32 },
    AuditRequiresAdmin { peer_uid: u32 },
}

impl AuthzError {
    pub const fn kind(&self) -> Kind {
        match self {
            Self::NotALauncher { .. } => Kind::AuthzNotALauncher,
            Self::AuditRequiresAdmin { .. } => Kind::AuthzAuditRequiresAdmin,
        }
    }

    pub const fn code(&self) -> u8 {
        self.kind().exit_code()
    }

    pub fn message(&self) -> String {
        match self {
            Self::NotALauncher { peer_uid } => {
                format!("peer uid {peer_uid} is not authorized to use the launcher API")
            }
            Self::AuditRequiresAdmin { peer_uid } => {
                format!("peer uid {peer_uid} is not authorized to read broker audit records")
            }
        }
    }

    pub const fn remediation(&self) -> &'static str {
        self.kind().remediation()
    }

    pub const fn owning_command(&self) -> &'static str {
        self.kind().owning_command()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    VersionMismatch {
        client: SemverRange,
        server: Version,
    },
    FrameTooLarge {
        received: u64,
        limit: u64,
    },
    UnknownField {
        type_name: &'static str,
        field: String,
    },
    IfNameInvalid {
        reason: IfNameError,
    },
    MalformedJson {
        type_name: &'static str,
        opaque_reason: String,
    },
    GuestShellDisabled,
}

impl WireError {
    pub const fn kind(&self) -> Kind {
        match self {
            Self::VersionMismatch { .. } => Kind::WireVersionMismatch,
            Self::FrameTooLarge { .. } => Kind::WireFrameTooLarge,
            Self::UnknownField { .. } => Kind::WireUnknownField,
            Self::IfNameInvalid { .. } => Kind::WireIfNameInvalid,
            Self::MalformedJson { .. } => Kind::WireMalformedJson,
            Self::GuestShellDisabled => Kind::GuestShellDisabled,
        }
    }

    pub const fn code(&self) -> u8 {
        self.kind().exit_code()
    }

    pub fn message(&self) -> String {
        match self {
            Self::VersionMismatch { client, server } => {
                format!(
                    "client version requirement {client} does not include server version {server}"
                )
            }
            Self::FrameTooLarge { received, limit } => {
                format!("received frame of {received} bytes exceeds the {limit}-byte limit")
            }
            Self::UnknownField { type_name, field } => {
                format!("{type_name} rejected unknown field `{field}`")
            }
            Self::IfNameInvalid { reason } => {
                format!("wire interface name is invalid: {reason}")
            }
            Self::MalformedJson {
                type_name,
                opaque_reason,
            } => format!(
                "{type_name} could not be decoded from JSON (opaque reason: {opaque_reason})"
            ),
            Self::GuestShellDisabled => {
                "persistent guest shell is not available for this VM".to_owned()
            }
        }
    }

    pub const fn remediation(&self) -> &'static str {
        self.kind().remediation()
    }

    pub const fn owning_command(&self) -> &'static str {
        self.kind().owning_command()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerError {
    Unimplemented {
        operation: BrokerOp,
        target_wave: u8,
    },
    ValidationFailed {
        what: String,
    },
}

impl BrokerError {
    pub const fn kind(&self) -> Kind {
        match self {
            Self::Unimplemented { .. } => Kind::BrokerUnimplemented,
            Self::ValidationFailed { .. } => Kind::BrokerValidationFailed,
        }
    }

    pub const fn code(&self) -> u8 {
        self.kind().exit_code()
    }

    pub fn message(&self) -> String {
        match self {
            Self::Unimplemented {
                operation,
                target_wave,
            } => {
                let _ = target_wave;
                format!("broker operation {operation} is not implemented in this build")
            }
            Self::ValidationFailed { what } => {
                format!("broker validation failed for opaque target {what}")
            }
        }
    }

    pub const fn remediation(&self) -> &'static str {
        self.kind().remediation()
    }

    pub const fn owning_command(&self) -> &'static str {
        self.kind().owning_command()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    ParseError {
        artifact: &'static str,
        opaque_reason: String,
    },
    /// Bundle's emitted manifestVersion is not what the daemon supports.
    /// Distinct kind from ParseError so
    /// operators can correlate the failure to a stale bundle render
    /// rather than a parse-side regression.
    VersionMismatch {
        artifact: &'static str,
        opaque_reason: String,
    },
}

impl ManifestError {
    pub const fn kind(&self) -> Kind {
        match self {
            Self::ParseError { .. } => Kind::ManifestParseError,
            Self::VersionMismatch { .. } => Kind::ManifestVersionMismatch,
        }
    }

    pub const fn code(&self) -> u8 {
        self.kind().exit_code()
    }

    pub fn message(&self) -> String {
        match self {
            Self::ParseError {
                artifact,
                opaque_reason,
            } => format!(
                "could not parse manifest artifact {artifact} (opaque reason: {opaque_reason})"
            ),
            Self::VersionMismatch {
                artifact,
                opaque_reason,
            } => format!(
                "manifest {artifact} declared an incompatible manifestVersion (opaque reason: {opaque_reason})"
            ),
        }
    }

    pub const fn remediation(&self) -> &'static str {
        self.kind().remediation()
    }

    pub const fn owning_command(&self) -> &'static str {
        self.kind().owning_command()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InternalError {
    Io { opaque_reason: String },
}

impl InternalError {
    pub const fn kind(&self) -> Kind {
        match self {
            Self::Io { .. } => Kind::InternalIo,
        }
    }

    pub const fn code(&self) -> u8 {
        self.kind().exit_code()
    }

    pub fn message(&self) -> String {
        match self {
            Self::Io { opaque_reason } => {
                format!("an internal I/O step failed (opaque reason: {opaque_reason})")
            }
        }
    }

    pub const fn remediation(&self) -> &'static str {
        self.kind().remediation()
    }

    pub const fn owning_command(&self) -> &'static str {
        self.kind().owning_command()
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, PartialOrd, Ord,
)]
pub enum BrokerOp {
    ApplyNftables,
    ApplyNmUnmanaged,
    ApplyRoute,
    ApplySysctl,
    BindUnixSocket,
    CreateOrReconcileUsersGroups,
    CreatePersistentTap,
    CreateTapFd,
    DelegateCgroupV2,
    ExportBrokerAudit,
    InjectSecretById,
    LaunchMinijailChild,
    ModprobeIfAllowed,
    OpenCgroupDir,
    OpenDevice,
    OpenFuse,
    OpenKvm,
    OpenVhostNet,
    PauseBroker,
    PrepareRuntimeDir,
    PrepareStateDir,
    PrepareStoreView,
    ReadSecretById,
    ResumeBroker,
    RotateSecretById,
    SetBridgePortFlags,
    SetSocketAcl,
    SetupMountNamespace,
    UpdateHostsFile,
    UsbipBind,
    UsbipProxyReconcile,
    UsbipUnbind,
    ValidateBundle,
}

impl fmt::Display for BrokerOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Version(String);

impl Version {
    pub fn new(value: impl Into<String>) -> Result<Self, semver::Error> {
        let value = value.into();
        semver::Version::parse(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Version {
    type Err = semver::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Version {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Version {
    fn schema_name() -> String {
        "Version".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        string_schema(
            "Semantic version string used by the daemon and clients, for example `0.4.0`.",
            None,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SemverRange(String);

impl SemverRange {
    pub fn new(value: impl Into<String>) -> Result<Self, semver::Error> {
        let value = value.into();
        semver::VersionReq::parse(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn allows(&self, version: &Version) -> bool {
        semver::VersionReq::parse(self.as_str())
            .expect("validated semver range")
            .matches(&semver::Version::parse(version.as_str()).expect("validated version"))
    }
}

impl FromStr for SemverRange {
    type Err = semver::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for SemverRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for SemverRange {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SemverRange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for SemverRange {
    fn schema_name() -> String {
        "SemverRange".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        string_schema(
            "Semantic version requirement string, for example `>=0.4.0, <0.5.0`.",
            None,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorEnvelope {
    kind: Kind,
    code: u8,
    message: String,
    remediation: String,
    docs_anchor: String,
    owning_command: String,
}

impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        ErrorEnvelope {
            kind: self.kind(),
            code: self.code(),
            message: self.message(),
            remediation: self.remediation().to_owned(),
            docs_anchor: self.docs_anchor(),
            owning_command: self.owning_command().to_owned(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Error {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let envelope = ErrorEnvelope::deserialize(deserializer)?;
        if envelope.code != envelope.kind.exit_code() {
            return Err(serde::de::Error::custom(format!(
                "error envelope code {} does not match kind {}",
                envelope.code, envelope.kind
            )));
        }
        Ok(Self::PublicEnvelope(envelope))
    }
}

impl JsonSchema for Error {
    fn schema_name() -> String {
        "Error".to_owned()
    }

    fn json_schema(r#gen: &mut SchemaGenerator) -> Schema {
        ErrorEnvelope::json_schema(r#gen)
    }
}

/// Bundle artifact tamper-resistance check failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleError {
    /// `reason` is a short machine-readable slug:
    /// `"symlink"`, `"not-regular-file"`, `"owner"`, `"mode"`, `"hash"`.
    Tampered { path: PathBuf, reason: String },
}

impl BundleError {
    pub const fn kind(&self) -> Kind {
        match self {
            Self::Tampered { .. } => Kind::BundleTampered,
        }
    }

    pub const fn code(&self) -> u8 {
        self.kind().exit_code()
    }

    pub fn message(&self) -> String {
        match self {
            Self::Tampered { path, reason } => format!(
                "bundle artifact {} failed tamper-resistance check: {reason}",
                path.display()
            ),
        }
    }

    pub const fn remediation(&self) -> &'static str {
        self.kind().remediation()
    }

    pub const fn owning_command(&self) -> &'static str {
        self.kind().owning_command()
    }
}

fn string_schema(description: &str, pattern: Option<&str>) -> Schema {
    let mut object = SchemaObject {
        instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
        string: Some(Box::new(StringValidation {
            max_length: None,
            min_length: None,
            pattern: pattern.map(ToOwned::to_owned),
        })),
        ..Default::default()
    };
    object.metadata = Some(Box::new(Metadata {
        description: Some(description.to_owned()),
        ..Default::default()
    }));
    Schema::Object(object)
}

#[cfg(test)]
mod tests {
    use super::{BrokerOp, Error, Kind, SemverRange, Version};
    use std::collections::BTreeSet;

    #[test]
    fn error_serializes_as_operator_envelope() {
        let error = Error::broker_unimplemented(BrokerOp::CreateTapFd, 3);
        let json = serde_json::to_value(&error).expect("error serializes");
        assert_eq!(json["kind"], "broker-unimplemented");
        assert_eq!(json["code"], 30);
        assert_eq!(json["owningCommand"], "daemon-api/broker");
        assert_eq!(
            json["docsAnchor"],
            "docs/reference/error-codes.md#broker-unimplemented"
        );
    }

    #[test]
    fn kind_serializes_as_leaf_discriminant() {
        let json = serde_json::to_value(Kind::AuthzNotALauncher).expect("kind serializes");
        assert_eq!(json, serde_json::json!("authz-not-a-launcher"));
    }

    #[test]
    fn all_kinds_records_are_unique_and_operator_visible() {
        let records = Error::all_kinds();
        // Kind::ManifestVersionMismatch is part of the public table.
        assert_eq!(records.len(), 37);

        let mut codes = BTreeSet::new();
        let mut anchors = BTreeSet::new();
        let mut discriminants = BTreeSet::new();
        for record in records {
            assert!((10..=99).contains(&record.exit_code));
            assert!(codes.insert(record.exit_code));
            assert!(anchors.insert(record.docs_anchor));
            assert!(discriminants.insert(record.kind.discriminant()));
            assert!(record.docs_anchor.starts_with('#'));
            assert_eq!(record.kind.docs_anchor(), record.docs_anchor);
        }
    }

    #[test]
    fn semver_range_matches_valid_server_version() {
        let range = SemverRange::new(">=0.4.0, <0.5.0").expect("range parses");
        let version = Version::new("0.4.1").expect("version parses");
        assert!(range.allows(&version));
    }

    /// Path-like substring: at least two slash-separated segments.
    fn path_regex() -> regex::Regex {
        regex::Regex::new(r"(/[a-zA-Z][a-zA-Z0-9_.-]*){2,}").unwrap()
    }

    fn assert_no_path_in_message(kind_label: &str, message: &str) {
        let re = path_regex();
        assert!(
            !re.is_match(message),
            "{kind_label}: core Error message leaks a host path: {message:?}"
        );
    }

    #[test]
    fn core_error_messages_never_contain_host_paths() {
        use super::{BrokerOp, Error, SemverRange, Version};
        use crate::host::IfNameError;

        let cases: Vec<(Error, &str)> = vec![
            (Error::not_a_launcher(1000), "authz-not-a-launcher"),
            (
                Error::audit_requires_admin(1000),
                "authz-audit-requires-admin",
            ),
            (
                Error::version_mismatch(
                    SemverRange::new(">=0.4.0, <0.5.0").unwrap(),
                    Version::new("0.5.1").unwrap(),
                ),
                "wire-version-mismatch",
            ),
            (
                Error::frame_too_large(2_000_000, 1_048_576),
                "wire-frame-too-large",
            ),
            (
                Error::unknown_field("HelloRequest", "badField"),
                "wire-unknown-field",
            ),
            (
                Error::if_name_invalid(IfNameError::TooLong),
                "wire-ifname-invalid",
            ),
            (
                Error::malformed_json("HelloRequest", "unexpected EOF at line 1 column 42"),
                "wire-malformed-json",
            ),
            (Error::guest_shell_disabled(), "guest-shell-disabled"),
            (
                Error::broker_unimplemented(BrokerOp::CreateTapFd, 3),
                "broker-unimplemented",
            ),
            (
                Error::broker_validation_failed("schema mismatch in bundle artifact"),
                "broker-validation-failed",
            ),
            (
                Error::manifest_parse_error("bundle.json", "invalid key at line 3 column 12"),
                "manifest-parse-error",
            ),
            (
                Error::internal_io("ENOENT during state-lock acquisition"),
                "internal-io",
            ),
            (
                Error::provider_misconfigured("corp-vm", "guestd agent not found"),
                "provider-misconfigured",
            ),
            // BundleTampered: the message contains the path, so the
            // assert_no_path_in_message check is deliberately skipped for it.
        ];

        for (error, expected_kind) in &cases {
            assert_eq!(error.kind().discriminant(), *expected_kind, "kind mismatch");
            let message = error.message();
            assert_no_path_in_message(expected_kind, &message);
        }

        // BundleTampered is exercised separately since its message intentionally
        // carries the artifact path for operator diagnostics.
        let tamper =
            Error::bundle_tampered(std::path::PathBuf::from("/etc/d2b/bundle.json"), "hash");
        assert_eq!(tamper.kind().discriminant(), "bundle-tampered");
        assert_eq!(tamper.code(), 60);
    }

    #[test]
    fn all_kinds_count_is_thirty_seven() {
        // Kind::ManifestVersionMismatch is part of the public table.
        assert_eq!(Error::all_kinds().len(), 37);
    }
}
