//! The placement and device configuration one Guest frontend is started with.
//!
//! A Guest agent cannot name its own Zone, mint its own enrollment, or choose
//! the ZoneLink it joins: the allocator places it. Everything this module reads
//! is that placement, handed to the process by whoever started it - the
//! Process controller in the v3 target, the transitional Guest unit before it.
//! Nothing here is a secret: a PSK reaches the Guest as an issuance ordinal and
//! a lifetime, never as bytes, and every fingerprint is a digest.

use std::path::PathBuf;

use d2b_contracts_resource::v3::identity::ReconnectGeneration;
use d2b_contracts_zone_session::v3::zone_routing::{ZoneLabelId, ZonePath, ZoneTreeEdge};
use d2b_contracts_zone_session::v3::zone_session::ZoneEnrollmentIdentity;
use d2b_provider_toolkit::{GuestError, GuestPlacement};

use crate::link::{SK_VSOCK_PORT, VsockAllocatorLink};

/// Maximum environment value length this configuration accepts.
const MAX_ENV_VALUE_BYTES: usize = 256;

/// The placement and device facts one frontend process was started with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The VM this frontend runs in, used to name its virtual HID device.
    pub vm_id: String,
    /// The allocator endpoint to enroll with.
    pub link: VsockAllocatorLink,
    /// The `/dev/uhid` path to create the virtual device on.
    pub uhid_path: PathBuf,
    /// The placement the allocator minted for this agent.
    pub placement: PlacementConfig,
}

/// The placement facts, as the allocator handed them to the process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacementConfig {
    /// The link identity the enrollment names.
    pub identity: ZoneEnrollmentIdentity,
    /// The allocator's issuance ordinal for the single-use PSK.
    pub psk_issuance: u64,
    /// The declared lifetime of that issuance, in milliseconds.
    pub psk_ttl_ms: u64,
    /// When the allocator issued it, in Unix milliseconds.
    pub psk_issued_at_unix_ms: u64,
    /// This frontend's own static-key fingerprint, as the allocator pinned it.
    pub static_key_fingerprint: [u8; 32],
}

impl PlacementConfig {
    /// Lower the placement into the toolkit's enrollment placement.
    pub fn into_placement(self) -> Result<GuestPlacement, GuestError> {
        GuestPlacement::new(
            self.identity,
            self.psk_issuance,
            self.psk_ttl_ms,
            self.psk_issued_at_unix_ms,
            self.static_key_fingerprint,
        )
    }
}

impl Config {
    /// Read the configuration from the process environment.
    ///
    /// Every required value is named in the refusal, so an unplaced frontend
    /// fails closed with the exact variable it is missing rather than starting
    /// without an enrollment.
    pub fn from_env() -> Result<Self, String> {
        let vm_id = required("D2B_SK_VM_ID")?;
        let vsock_port = match optional("D2B_SK_VSOCK_PORT") {
            Some(value) => value
                .parse::<u32>()
                .map_err(|error| format!("D2B_SK_VSOCK_PORT: {error}"))?,
            None => SK_VSOCK_PORT,
        };
        let vsock_cid = match optional("D2B_SK_VSOCK_CID") {
            Some(value) => value
                .parse::<u32>()
                .map_err(|error| format!("D2B_SK_VSOCK_CID: {error}"))?,
            None => crate::link::VSOCK_HOST_CID,
        };
        let uhid_path = optional("D2B_SK_UHID_PATH").unwrap_or_else(|| "/dev/uhid".to_owned());

        let parent_zone = zone_path(&required("D2B_SK_PARENT_ZONE")?, "D2B_SK_PARENT_ZONE")?;
        let guest_zone = zone_path(&required("D2B_SK_GUEST_ZONE")?, "D2B_SK_GUEST_ZONE")?;
        if parent_zone == guest_zone {
            return Err("D2B_SK_GUEST_ZONE must differ from D2B_SK_PARENT_ZONE".to_owned());
        }
        let edge = ZoneTreeEdge::new(parent_zone, guest_zone)
            .map_err(|_| "D2B_SK_GUEST_ZONE must be a direct child of D2B_SK_PARENT_ZONE".to_owned())?;
        let identity = ZoneEnrollmentIdentity {
            zone_link_uid: d2b_contracts_resource::v3::ResourceUid::parse(required(
                "D2B_SK_ZONE_LINK_UID",
            )?)
            .map_err(|error| format!("D2B_SK_ZONE_LINK_UID: {error}"))?,
            edge,
            controller_generation:
                d2b_contracts_zone_session::v3::zone_routing::ZoneLinkControllerGeneration::parse(
                    required("D2B_SK_CONTROLLER_GENERATION")?,
                )
                .map_err(|error| format!("D2B_SK_CONTROLLER_GENERATION: {error}"))?,
            reconnect_generation: ReconnectGeneration::new(
                required("D2B_SK_RECONNECT_GENERATION")?
                    .parse::<u64>()
                    .map_err(|error| format!("D2B_SK_RECONNECT_GENERATION: {error}"))?,
            )
            .map_err(|_| "D2B_SK_RECONNECT_GENERATION must be nonzero".to_owned())?,
            schema_fingerprint: digest("D2B_SK_SCHEMA_FINGERPRINT")?,
        };

        Ok(Self {
            vm_id,
            link: VsockAllocatorLink::new(vsock_cid, vsock_port),
            uhid_path: PathBuf::from(uhid_path),
            placement: PlacementConfig {
                identity,
                psk_issuance: number("D2B_SK_PSK_ISSUANCE")?,
                psk_ttl_ms: number("D2B_SK_PSK_TTL_MS")?,
                psk_issued_at_unix_ms: number("D2B_SK_PSK_ISSUED_AT_UNIX_MS")?,
                static_key_fingerprint: digest("D2B_SK_STATIC_KEY_FINGERPRINT")?,
            },
        })
    }
}

fn required(name: &str) -> Result<String, String> {
    match optional(name) {
        Some(value) => Ok(value),
        None => Err(format!("{name} is required")),
    }
}

fn optional(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() && value.len() <= MAX_ENV_VALUE_BYTES => Some(value),
        _ => None,
    }
}

fn number(name: &str) -> Result<u64, String> {
    required(name)?
        .parse::<u64>()
        .map_err(|error| format!("{name}: {error}"))
}

fn digest(name: &str) -> Result<[u8; 32], String> {
    digest_value(name, &required(name)?)
}

fn digest_value(name: &str, value: &str) -> Result<[u8; 32], String> {
    let bytes = value.as_bytes();
    if bytes.len() != 64 {
        return Err(format!("{name} must be 64 lower-case hex characters"));
    }
    let mut digest = [0u8; 32];
    for (index, pair) in bytes.chunks_exact(2).enumerate() {
        let high = hex(pair[0]).ok_or_else(|| format!("{name} is not lower-case hex"))?;
        let low = hex(pair[1]).ok_or_else(|| format!("{name} is not lower-case hex"))?;
        digest[index] = (high << 4) | low;
    }
    if digest == [0; 32] {
        return Err(format!("{name} must not be all zero"));
    }
    Ok(digest)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// Parse one `/`-separated Zone path, most specific first.
fn zone_path(value: &str, name: &str) -> Result<ZonePath, String> {
    let mut labels = Vec::new();
    for label in value.split('/') {
        labels.push(
            ZoneLabelId::parse(label)
                .map_err(|_| format!("{name} is not a valid Zone label path"))?,
        );
    }
    ZonePath::new(labels).map_err(|_| format!("{name} is not a valid Zone label path"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest_of(value: &str) -> Result<[u8; 32], String> {
        digest_value("TEST", value)
    }

    #[test]
    fn digests_are_lower_case_hex_and_never_zero() {
        assert_eq!(
            digest_of("33".repeat(32).as_str()),
            Ok([0x33; 32]),
            "a well-formed digest decodes"
        );
        assert!(
            digest_of("3g".repeat(32).as_str()).is_err(),
            "upper-case or non-hex characters are refused"
        );
        assert!(digest_of("33").is_err(), "a short digest is refused");
        assert!(digest_of(&"00".repeat(32)).is_err(), "an all-zero digest is refused");
    }

    fn valid_digest() -> String {
        "33".repeat(32)
    }

    fn valid_vars() -> Vec<(&'static str, String)> {
        vec![
            ("D2B_SK_VM_ID", "vm-1".to_owned()),
            ("D2B_SK_VSOCK_PORT", "5050".to_owned()),
            ("D2B_SK_VSOCK_CID", "3".to_owned()),
            ("D2B_SK_UHID_PATH", "/dev/uhid".to_owned()),
            ("D2B_SK_PARENT_ZONE", "k0".to_owned()),
            ("D2B_SK_GUEST_ZONE", "k1/k0".to_owned()),
            (
                "D2B_SK_ZONE_LINK_UID",
                "123e4567-e89b-42d3-a456-426614174000".to_owned(),
            ),
            ("D2B_SK_CONTROLLER_GENERATION", "gen-1".to_owned()),
            ("D2B_SK_RECONNECT_GENERATION", "1".to_owned()),
            ("D2B_SK_SCHEMA_FINGERPRINT", valid_digest()),
            ("D2B_SK_PSK_ISSUANCE", "7".to_owned()),
            ("D2B_SK_PSK_TTL_MS", "30000".to_owned()),
            ("D2B_SK_PSK_ISSUED_AT_UNIX_MS", "1000".to_owned()),
            ("D2B_SK_STATIC_KEY_FINGERPRINT", valid_digest()),
        ]
    }

    /// Re-execute this test binary with a controlled environment so
    /// `Config::from_env` sees exactly the placement under test. The crate
    /// forbids `unsafe` (no `std::env::set_var`), so the environment is
    /// injected through the child process instead.
    fn run_child(case: &str, vars: &[(&str, String)]) -> std::process::Output {
        let exe = std::env::current_exe().expect("test binary");
        let mut command = std::process::Command::new(exe);
        command
            .arg("--exact")
            .arg("config::tests::from_env_child_probe")
            .arg("--nocapture")
            .env("D2B_SK_TEST_CHILD", "1")
            .env("D2B_SK_TEST_CASE", case);
        for (name, value) in vars {
            command.env(name, value);
        }
        command.output().expect("spawn from_env child probe")
    }

    #[test]
    fn from_env_fails_closed_on_missing_or_invalid_placement() {
        // Every case is asserted inside the child probe: the child exits
        // non-zero exactly when the expected refusal did not happen.
        let cases: [(&str, Vec<(&str, String)>); 8] = [
            ("valid", valid_vars()),
            ("missing-vm-id", {
                let mut vars = valid_vars();
                vars.retain(|(name, _)| *name != "D2B_SK_VM_ID");
                vars
            }),
            ("guest-equals-parent", {
                let mut vars = valid_vars();
                vars.iter_mut()
                    .find(|(name, _)| *name == "D2B_SK_GUEST_ZONE")
                    .expect("guest zone var")
                    .1 = "k0".to_owned();
                vars
            }),
            ("not-direct-child", {
                let mut vars = valid_vars();
                vars.iter_mut()
                    .find(|(name, _)| *name == "D2B_SK_GUEST_ZONE")
                    .expect("guest zone var")
                    .1 = "k2/k1/k0".to_owned();
                vars
            }),
            ("zero-reconnect", {
                let mut vars = valid_vars();
                vars.iter_mut()
                    .find(|(name, _)| *name == "D2B_SK_RECONNECT_GENERATION")
                    .expect("reconnect var")
                    .1 = "0".to_owned();
                vars
            }),
            ("invalid-port", {
                let mut vars = valid_vars();
                vars.iter_mut()
                    .find(|(name, _)| *name == "D2B_SK_VSOCK_PORT")
                    .expect("port var")
                    .1 = "not-a-port".to_owned();
                vars
            }),
            ("invalid-digest", {
                let mut vars = valid_vars();
                vars.iter_mut()
                    .find(|(name, _)| *name == "D2B_SK_SCHEMA_FINGERPRINT")
                    .expect("digest var")
                    .1 = "not-hex".to_owned();
                vars
            }),
            ("empty-parent", {
                let mut vars = valid_vars();
                vars.iter_mut()
                    .find(|(name, _)| *name == "D2B_SK_PARENT_ZONE")
                    .expect("parent var")
                    .1 = String::new();
                vars
            }),
        ];
        for (case, vars) in cases {
            let output = run_child(case, &vars);
            assert!(
                output.status.success(),
                "child probe {case} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn from_env_child_probe() {
        if std::env::var("D2B_SK_TEST_CHILD").is_err() {
            return; // no-op unless spawned by from_env_fails_closed_on_missing_or_invalid_placement
        }
        let case = std::env::var("D2B_SK_TEST_CASE").expect("case name");
        match case.as_str() {
            "valid" => {
                let config = Config::from_env().expect("all values valid");
                assert_eq!(config.vm_id, "vm-1");
                assert_eq!(config.link.port(), 5050);
                assert_eq!(config.placement.psk_issuance, 7);
                assert_eq!(config.placement.identity.reconnect_generation.get(), 1);
            }
            "missing-vm-id" => {
                let error = Config::from_env().expect_err("missing vm id");
                assert!(error.contains("D2B_SK_VM_ID is required"), "{error}");
            }
            "guest-equals-parent" => {
                let error = Config::from_env().expect_err("guest equals parent");
                assert!(error.contains("must differ"), "{error}");
            }
            "not-direct-child" => {
                let error = Config::from_env().expect_err("guest not a direct child");
                assert!(error.contains("direct child"), "{error}");
            }
            "zero-reconnect" => {
                let error = Config::from_env().expect_err("zero reconnect generation");
                assert!(error.contains("must be nonzero"), "{error}");
            }
            "invalid-port" => {
                let error = Config::from_env().expect_err("invalid vsock port");
                assert!(error.contains("D2B_SK_VSOCK_PORT"), "{error}");
            }
            "invalid-digest" => {
                let error = Config::from_env().expect_err("invalid digest");
                assert!(error.contains("D2B_SK_SCHEMA_FINGERPRINT"), "{error}");
            }
            "empty-parent" => {
                let error = Config::from_env().expect_err("empty value is missing");
                assert!(error.contains("D2B_SK_PARENT_ZONE is required"), "{error}");
            }
            other => panic!("unknown from_env child case: {other}"),
        }
    }

    #[test]
    fn zone_paths_are_label_paths_most_specific_first() {
        let path = zone_path("k2/k1/k0", "TEST").expect("a valid path");
        let labels: Vec<&str> = path.labels().iter().map(|label| label.as_str()).collect();
        assert_eq!(
            labels,
            ["k2", "k1", "k0"],
            "the path keeps its most-specific-first order"
        );
        assert!(zone_path("", "TEST").is_err());
        assert!(
            zone_path("K2/k0", "TEST").is_err(),
            "labels are lower-case"
        );
    }
}
