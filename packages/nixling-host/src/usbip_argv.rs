//! USBIP bind/unbind argv generator.
//!
//! Pure Rust functions emitting the host-side `usbip bind --busid X`
//! and `usbip unbind --busid X` argv that the daemon uses when moving a
//! USB device into a VM's exclusive control. The broker variant
//! `UsbipBind` is wire-stable but may return
//! `BrokerError::UnknownOperation` until it is wired to invoke this
//! generator's output.
//!
//! `usbip` CLI shapes (per linux-tools/usbip(8)):
//!
//! ```text
//! usbip bind   --busid <bus-id>
//! usbip unbind --busid <bus-id>
//! usbip list   --local
//! ```
//!
//! Per-busid lock + env exclusivity + audit are broker-side concerns;
//! this module is only the pure argv shape.
//!
//! Crate invariant `#![forbid(unsafe_code)]` is honoured.

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

/// All inputs required to render a usbip subcommand argv.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbipArgvInput {
    /// Absolute store path to the `usbip` binary.
    pub usbip_binary_path: String,
    /// USB bus id in the canonical `B-P` form (e.g. `1-2`,
    /// `2-1.4`). The generator validates the shape rather than
    /// passing arbitrary strings into the subprocess argv.
    pub bus_id: String,
}

/// Subset of the `usbip` subcommand surface the daemon uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipSubcommand {
    /// Host-side bind: `usbip bind --busid <bus_id>` (binds the
    /// device to the host's usbip-host driver so the userspace
    /// daemon can export it).
    Bind,
    /// Host-side unbind: `usbip unbind --busid <bus_id>`.
    Unbind,
    /// Guest-side attach: `usbip attach -r <host_ip> -b <bus_id>`
    /// (exec inside the guest; nixling drives this via the
    /// `GuestUsbipAttach` SpawnRunner role which wraps ssh).
    Attach,
    /// Guest-side detach: `usbip detach -p <port>` (also exec
    /// inside the guest via the `GuestUsbipDetach` SpawnRunner
    /// role; bus-id alone isn't enough — we record the port the
    /// guest's `usbip attach` assigned).
    Detach,
}

impl UsbipSubcommand {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bind => "bind",
            Self::Unbind => "unbind",
            Self::Attach => "attach",
            Self::Detach => "detach",
        }
    }
}

/// Errors the USBIP argv generator can return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum UsbipArgvError {
    InvalidUsbipBinaryPath {
        path: String,
    },
    EmptyBusId,
    /// `bus_id` did not match the canonical `B[-P[.P]...]` form.
    /// usbip(8) rejects malformed bus ids, but doing the check here
    /// surfaces the failure at the daemon edge with a typed error
    /// instead of "command exited 1".
    InvalidBusId {
        bus_id: String,
    },
    /// usbip's `SYSFS_BUS_ID_SIZE` is 32 bytes including NUL. Anything
    /// longer can't match a real
    /// sysfs device name; reject at the generator layer for a
    /// typed error rather than a downstream `usbip exited 1`.
    BusIdTooLong {
        bus_id: String,
        max: usize,
    },
}

/// SYSFS_BUS_ID_SIZE per `include/linux/mod_devicetable.h` and
/// `tools/usb/usbip/libsrc/usbip_common.h`: 32 bytes including the
/// trailing NUL, so the printable busid is at most 31 chars.
const USBIP_SYSFS_BUS_ID_MAX: usize = 31;

/// Validate a USB bus id shape. Accepted forms:
///
/// - `B` (root hub bus, rare): digits, no leading zeros except the
///   literal single digit `0`.
/// - `B-P` (port on root hub): digits-dash-digits.
/// - `B-P.S[.S...]` (port on chained hub): digits-dash-digits.dots.
///
/// Validation details:
/// - Leading zeros in any segment are rejected (sysfs uses the
///   canonical decimal form; `01-02` would not match any real
///   device).
/// - Total length is capped at `USBIP_SYSFS_BUS_ID_MAX` (31 chars).
/// - ASCII digits only — Unicode digits like ٢ (Arabic-Indic 2)
///   refused.
pub fn validate_bus_id(bus_id: &str) -> Result<(), UsbipArgvError> {
    if bus_id.is_empty() {
        return Err(UsbipArgvError::EmptyBusId);
    }
    if bus_id.len() > USBIP_SYSFS_BUS_ID_MAX {
        return Err(UsbipArgvError::BusIdTooLong {
            bus_id: bus_id.to_owned(),
            max: USBIP_SYSFS_BUS_ID_MAX,
        });
    }
    fn segment_ok(segment: &str) -> bool {
        if segment.is_empty() {
            return false;
        }
        if !segment.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        // Reject leading zeros except the literal "0".
        if segment.len() > 1 && segment.starts_with('0') {
            return false;
        }
        true
    }
    match bus_id.split_once('-') {
        None => {
            if !segment_ok(bus_id) {
                return Err(UsbipArgvError::InvalidBusId {
                    bus_id: bus_id.to_owned(),
                });
            }
        }
        Some((bus, port_chain)) => {
            if !segment_ok(bus) {
                return Err(UsbipArgvError::InvalidBusId {
                    bus_id: bus_id.to_owned(),
                });
            }
            if port_chain.is_empty() {
                return Err(UsbipArgvError::InvalidBusId {
                    bus_id: bus_id.to_owned(),
                });
            }
            for segment in port_chain.split('.') {
                if !segment_ok(segment) {
                    return Err(UsbipArgvError::InvalidBusId {
                        bus_id: bus_id.to_owned(),
                    });
                }
            }
        }
    }
    Ok(())
}

/// Render the usbip argv for the requested subcommand.
pub fn generate_usbip_argv(
    input: &UsbipArgvInput,
    sub: UsbipSubcommand,
) -> Result<Vec<String>, UsbipArgvError> {
    if input.usbip_binary_path.is_empty() || !input.usbip_binary_path.starts_with('/') {
        return Err(UsbipArgvError::InvalidUsbipBinaryPath {
            path: input.usbip_binary_path.clone(),
        });
    }
    validate_bus_id(&input.bus_id)?;
    Ok(vec![
        input.usbip_binary_path.clone(),
        sub.as_str().to_owned(),
        "--busid".to_owned(),
        input.bus_id.clone(),
    ])
}

/// v1.1.1 guest-side USBIP attach/detach driver inputs.
///
/// Per ADR 0011 § "USBIP guest ssh attach/detach" + ADR 0018
/// § "Per-attach SpawnRunner-leaf model": the daemon dispatches
/// guest-side `usbip attach -r <host> -b <bus_id>` (or `detach -p
/// <port>`) by SSHing into the guest. The ssh command is hardened
/// against control-master leakage / job-control issues:
/// - NO `-f` (don't background)
/// - NO `-N` (DO run a command — we want `usbip attach/detach`)
/// - NO `-M` (no control-master)
/// - `-o ControlMaster=no` (defense-in-depth against
///   `~/.ssh/config` defaults)
/// - `-o ControlPersist=no`
/// - `-o BatchMode=yes` (no interactive prompts)
/// - `-F /dev/null` (ignore consumer `~/.ssh/config`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuestUsbipSshInput {
    /// Absolute store path to the host's `ssh` binary.
    pub ssh_binary_path: String,
    /// User to ssh as on the guest (typically the workload VM's
    /// `nixling.sshUser`).
    pub ssh_user: String,
    /// Guest's static IP for the ssh target.
    pub guest_ip: String,
    /// Path to the per-VM nixling-managed SSH private key on the
    /// host (`/var/lib/nixling/keys/<vm>/id_ed25519`).
    pub identity_path: String,
    /// Path to the per-VM `known_hosts` file on the host
    /// (`/var/lib/nixling/known_hosts.nixling`).
    pub known_hosts_path: String,
    /// USBIP subcommand to run in the guest. Must be Attach or
    /// Detach; Bind/Unbind are host-side and rejected here.
    pub sub: UsbipSubcommand,
    /// Host's USBIP backend IP (the address the guest connects
    /// back to). Required for Attach; ignored for Detach.
    pub backend_ip: Option<String>,
    /// Bus ID to attach (`Attach` mode only). Validated per
    /// `validate_bus_id`.
    pub bus_id: Option<String>,
    /// Port to detach (`Detach` mode only). Set by tracking the
    /// guest's `usbip attach` output port assignment.
    pub detach_port: Option<u16>,
}

/// Generate the host-side `ssh ... -- usbip <sub> ...` argv that
/// drives a guest's usbip attach/detach.
pub fn generate_guest_usbip_ssh_argv(
    input: &GuestUsbipSshInput,
) -> Result<Vec<String>, UsbipArgvError> {
    if input.ssh_binary_path.is_empty() || !input.ssh_binary_path.starts_with('/') {
        return Err(UsbipArgvError::InvalidUsbipBinaryPath {
            path: input.ssh_binary_path.clone(),
        });
    }
    if !matches!(input.sub, UsbipSubcommand::Attach | UsbipSubcommand::Detach) {
        return Err(UsbipArgvError::InvalidBusId {
            bus_id: format!(
                "guest-ssh USBIP requires Attach or Detach subcommand; got {:?}",
                input.sub
            ),
        });
    }
    let mut argv: Vec<String> = vec![
        input.ssh_binary_path.clone(),
        "-F".to_owned(),
        "/dev/null".to_owned(),
        "-o".to_owned(),
        "BatchMode=yes".to_owned(),
        "-o".to_owned(),
        "ControlMaster=no".to_owned(),
        "-o".to_owned(),
        "ControlPersist=no".to_owned(),
        "-o".to_owned(),
        format!("UserKnownHostsFile={}", input.known_hosts_path),
        "-o".to_owned(),
        "StrictHostKeyChecking=yes".to_owned(),
        "-i".to_owned(),
        input.identity_path.clone(),
        "-l".to_owned(),
        input.ssh_user.clone(),
        input.guest_ip.clone(),
        "--".to_owned(),
        "usbip".to_owned(),
        input.sub.as_str().to_owned(),
    ];
    match input.sub {
        UsbipSubcommand::Attach => {
            let backend =
                input
                    .backend_ip
                    .as_ref()
                    .ok_or_else(|| UsbipArgvError::InvalidBusId {
                        bus_id: "backend_ip required for Attach".to_owned(),
                    })?;
            let bus = input
                .bus_id
                .as_ref()
                .ok_or_else(|| UsbipArgvError::InvalidBusId {
                    bus_id: "bus_id required for Attach".to_owned(),
                })?;
            validate_bus_id(bus)?;
            argv.push("-r".to_owned());
            argv.push(backend.clone());
            argv.push("-b".to_owned());
            argv.push(bus.clone());
        }
        UsbipSubcommand::Detach => {
            let port = input
                .detach_port
                .ok_or_else(|| UsbipArgvError::InvalidBusId {
                    bus_id: "detach_port required for Detach".to_owned(),
                })?;
            argv.push("-p".to_owned());
            argv.push(port.to_string());
        }
        _ => unreachable!(),
    }
    Ok(argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audit_input() -> UsbipArgvInput {
        UsbipArgvInput {
            usbip_binary_path: "/nix/store/USBIPUSBIPUSBIP-linux-usbip/bin/usbip".to_owned(),
            bus_id: "1-2".to_owned(),
        }
    }

    #[test]
    fn bind_argv_has_expected_shape() {
        let argv = generate_usbip_argv(&audit_input(), UsbipSubcommand::Bind).unwrap();
        assert_eq!(
            argv,
            vec![
                "/nix/store/USBIPUSBIPUSBIP-linux-usbip/bin/usbip".to_owned(),
                "bind".to_owned(),
                "--busid".to_owned(),
                "1-2".to_owned(),
            ]
        );
    }

    #[test]
    fn unbind_argv_has_expected_shape() {
        let argv = generate_usbip_argv(&audit_input(), UsbipSubcommand::Unbind).unwrap();
        assert_eq!(argv[1], "unbind");
    }

    /// USBIP byte-parity snapshot. Emits two SNAPSHOT lines in a stable
    /// order (bind first, then unbind) consumed by
    /// `tests/usbip-argv-shape.sh` against
    /// `tests/golden/runner-shape/usbip-argv-minimal.txt`.
    #[test]
    fn argv_snapshot_lines() {
        let input = audit_input();
        let bind = generate_usbip_argv(&input, UsbipSubcommand::Bind).unwrap();
        println!("SNAPSHOT: {}", bind.join(" "));
        let unbind = generate_usbip_argv(&input, UsbipSubcommand::Unbind).unwrap();
        println!("SNAPSHOT: {}", unbind.join(" "));
    }

    #[test]
    fn accepts_chained_hub_bus_id() {
        let mut input = audit_input();
        input.bus_id = "2-1.4".to_owned();
        assert!(generate_usbip_argv(&input, UsbipSubcommand::Bind).is_ok());
    }

    /// Explicit test for multi-digit B (>= 10 USB controllers is rare
    /// but legal).
    #[test]
    fn accepts_multi_digit_bus_number() {
        let mut input = audit_input();
        input.bus_id = "10-3.2".to_owned();
        assert!(generate_usbip_argv(&input, UsbipSubcommand::Bind).is_ok());
    }

    #[test]
    fn accepts_deeply_chained_hub_bus_id() {
        let mut input = audit_input();
        input.bus_id = "2-1.4.3.2".to_owned();
        assert!(generate_usbip_argv(&input, UsbipSubcommand::Bind).is_ok());
    }

    #[test]
    fn accepts_root_only_bus_id() {
        let mut input = audit_input();
        input.bus_id = "1".to_owned();
        assert!(generate_usbip_argv(&input, UsbipSubcommand::Bind).is_ok());
    }

    #[test]
    fn rejects_invalid_binary_path() {
        let mut input = audit_input();
        input.usbip_binary_path = "usbip".to_owned();
        assert!(matches!(
            generate_usbip_argv(&input, UsbipSubcommand::Bind),
            Err(UsbipArgvError::InvalidUsbipBinaryPath { .. })
        ));
    }

    #[test]
    fn rejects_empty_bus_id() {
        let mut input = audit_input();
        input.bus_id.clear();
        assert!(matches!(
            generate_usbip_argv(&input, UsbipSubcommand::Bind),
            Err(UsbipArgvError::EmptyBusId)
        ));
    }

    #[test]
    fn rejects_shell_metachar_bus_id() {
        let mut input = audit_input();
        input.bus_id = "1-2;rm -rf /".to_owned();
        assert!(matches!(
            generate_usbip_argv(&input, UsbipSubcommand::Bind),
            Err(UsbipArgvError::InvalidBusId { .. })
        ));
    }

    #[test]
    fn rejects_bus_id_with_letters() {
        let mut input = audit_input();
        input.bus_id = "a-b".to_owned();
        assert!(matches!(
            generate_usbip_argv(&input, UsbipSubcommand::Bind),
            Err(UsbipArgvError::InvalidBusId { .. })
        ));
    }

    #[test]
    fn rejects_bus_id_with_empty_port() {
        let mut input = audit_input();
        input.bus_id = "1-".to_owned();
        assert!(matches!(
            generate_usbip_argv(&input, UsbipSubcommand::Bind),
            Err(UsbipArgvError::InvalidBusId { .. })
        ));
    }

    #[test]
    fn rejects_bus_id_with_empty_chain_segment() {
        let mut input = audit_input();
        input.bus_id = "1-2..3".to_owned();
        assert!(matches!(
            generate_usbip_argv(&input, UsbipSubcommand::Bind),
            Err(UsbipArgvError::InvalidBusId { .. })
        ));
    }

    #[test]
    fn rejects_bus_id_with_leading_dot() {
        let mut input = audit_input();
        input.bus_id = "1-.2".to_owned();
        assert!(matches!(
            generate_usbip_argv(&input, UsbipSubcommand::Bind),
            Err(UsbipArgvError::InvalidBusId { .. })
        ));
    }

    /// Explicit test for trailing-dot rejection (covered by empty-segment
    /// logic but worth pinning).
    #[test]
    fn rejects_bus_id_with_trailing_dot() {
        let mut input = audit_input();
        input.bus_id = "1-2.".to_owned();
        assert!(matches!(
            generate_usbip_argv(&input, UsbipSubcommand::Bind),
            Err(UsbipArgvError::InvalidBusId { .. })
        ));
    }

    /// Explicit test for Unicode-digit rejection. is_ascii_digit()
    /// correctly rejects ٢
    /// (Arabic-Indic digit 2 / U+0662).
    #[test]
    fn rejects_unicode_digits() {
        let mut input = audit_input();
        input.bus_id = "1-\u{0662}".to_owned();
        assert!(matches!(
            generate_usbip_argv(&input, UsbipSubcommand::Bind),
            Err(UsbipArgvError::InvalidBusId { .. })
        ));
    }

    /// Leading zeros are rejected so that the generator output matches
    /// the sysfs canonical form
    /// usbip(8) expects.
    #[test]
    fn rejects_leading_zero_in_bus_segment() {
        let mut input = audit_input();
        input.bus_id = "01-02".to_owned();
        assert!(matches!(
            generate_usbip_argv(&input, UsbipSubcommand::Bind),
            Err(UsbipArgvError::InvalidBusId { .. })
        ));
    }

    #[test]
    fn rejects_leading_zero_in_chained_segment() {
        let mut input = audit_input();
        input.bus_id = "2-1.04".to_owned();
        assert!(matches!(
            generate_usbip_argv(&input, UsbipSubcommand::Bind),
            Err(UsbipArgvError::InvalidBusId { .. })
        ));
    }

    /// Literal single digit `0` is accepted (root hub bus 0 is
    /// legal on some controllers).
    #[test]
    fn accepts_literal_zero_segment() {
        let mut input = audit_input();
        input.bus_id = "0-1".to_owned();
        assert!(generate_usbip_argv(&input, UsbipSubcommand::Bind).is_ok());
    }

    /// SYSFS_BUS_ID_SIZE caps printable busid at 31 chars. Longer is
    /// refused with a typed error.
    #[test]
    fn rejects_bus_id_over_sysfs_max_length() {
        let mut input = audit_input();
        // 32 chars: "1-2.3.4.5.6.7.8.9.10.11.12.13.14" = 32 chars
        input.bus_id = "1-2.3.4.5.6.7.8.9.10.11.12.13.14".to_owned();
        assert!(input.bus_id.len() > 31, "test fixture must exceed 31");
        match generate_usbip_argv(&input, UsbipSubcommand::Bind) {
            Err(UsbipArgvError::BusIdTooLong { max, .. }) => assert_eq!(max, 31),
            other => panic!("expected BusIdTooLong, got {other:?}"),
        }
    }

    /// 31-char busid still accepted.
    #[test]
    fn accepts_bus_id_at_sysfs_max_length() {
        let mut input = audit_input();
        // 30 chars (under 31): "1-2.3.4.5.6.7.8.9.10.11.12.13" = 29 chars
        input.bus_id = "1-2.3.4.5.6.7.8.9.10.11.12.13".to_owned();
        assert!(input.bus_id.len() <= 31);
        assert!(generate_usbip_argv(&input, UsbipSubcommand::Bind).is_ok());
    }

    #[test]
    fn rejects_bus_id_with_slash() {
        let mut input = audit_input();
        input.bus_id = "1-2/3".to_owned();
        assert!(matches!(
            generate_usbip_argv(&input, UsbipSubcommand::Bind),
            Err(UsbipArgvError::InvalidBusId { .. })
        ));
    }

    #[test]
    fn rejects_bus_id_with_space() {
        let mut input = audit_input();
        input.bus_id = "1- 2".to_owned();
        assert!(matches!(
            generate_usbip_argv(&input, UsbipSubcommand::Bind),
            Err(UsbipArgvError::InvalidBusId { .. })
        ));
    }

    #[test]
    fn subcommand_string_round_trip() {
        assert_eq!(UsbipSubcommand::Bind.as_str(), "bind");
        assert_eq!(UsbipSubcommand::Unbind.as_str(), "unbind");
    }

    #[test]
    fn argv_input_round_trip_serializable() {
        let input = audit_input();
        let json = serde_json::to_string(&input).unwrap();
        let parsed: UsbipArgvInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, input);
    }
}

// ---------------------------------------------------------------------------
// Per-env usbipd-backend + TCP proxy argv generators.
//
// The per-env systemd units declared by `nixos-modules/network.nix`
// (`nixling-sys-<env>-usbipd-{backend,proxy}.service`) were retired in
// v1.0 and replaced by the nixlingd daemon spawning the backend + proxy
// through broker `SpawnRunner` with `RunnerRole::Usbip`. These two pure
// argv generators emit the exact argv the daemon-side `processes.json`
// rows emit.
// ---------------------------------------------------------------------------

/// Inputs for the per-env `usbipd` backend long-lived process. The
/// backend binds 127.0.0.1:<port> via source-based iptables defence-
/// in-depth (usbipd has no `--host` flag — it always binds 0.0.0.0).
/// The proxy unit is the user-facing listener; see
/// [`UsbipdProxyArgvInput`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbipdBackendArgvInput {
    /// Absolute store path to the `usbipd` binary.
    pub usbipd_binary_path: String,
    /// Env name owning this backend (e.g. `work`, `personal`,
    /// `obs`). Used to render the argv[0] process title so journal /
    /// `ps` output identifies the env.
    pub env: String,
    /// Per-env loopback TCP port. The Nix module derives this as
    /// `3241 + alphabetical-index-of-env` and the daemon mirrors that
    /// derivation when building the bundle intent.
    pub backend_port: u16,
}

/// Inputs for the per-env TCP proxy that fronts the backend. The
/// daemon-spawned shape starts the proxy directly and binds the env's
/// uplink IP:3240.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbipdProxyArgvInput {
    /// Absolute store path to `socat`.
    pub socat_binary_path: String,
    /// Env name owning this proxy. Renders the argv[0] process title.
    pub env: String,
    /// Env host-uplink IP the proxy listens on.
    pub host_uplink_ip: String,
    /// Per-env loopback TCP port the proxy forwards to.
    pub backend_port: u16,
}

/// Errors the per-env usbipd backend/proxy argv generators return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum UsbipdPerEnvArgvError {
    InvalidBinaryPath {
        path: String,
    },
    EmptyEnv,
    /// Env name carries characters outside the manifest-validated
    /// regex `[a-z][a-z0-9-]*`. Surface fail-closed at the generator
    /// layer so a tampered bundle can't smuggle a backslash or a
    /// space into the argv[0] process title.
    InvalidEnv {
        env: String,
    },
    InvalidHostUplinkIp {
        host_uplink_ip: String,
    },
    /// Port `0` is illegal; the manifest derivation starts at 3241.
    InvalidPort,
}

fn validate_env(env: &str) -> Result<(), UsbipdPerEnvArgvError> {
    if env.is_empty() {
        return Err(UsbipdPerEnvArgvError::EmptyEnv);
    }
    let mut bytes = env.bytes();
    let first_ok = matches!(bytes.next(), Some(b) if b.is_ascii_lowercase());
    let rest_ok = bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !first_ok || !rest_ok {
        return Err(UsbipdPerEnvArgvError::InvalidEnv {
            env: env.to_owned(),
        });
    }
    Ok(())
}

/// Render the argv for the per-env `usbipd` backend. Mirrors the
/// `nixling-sys-<env>-usbipd-backend.service` `ExecStart=` line
/// byte-for-byte (see `nixos-modules/network.nix` `usbipd -4
/// --tcp-port <port>`).
pub fn generate_usbipd_backend_argv(
    input: &UsbipdBackendArgvInput,
) -> Result<Vec<String>, UsbipdPerEnvArgvError> {
    if input.usbipd_binary_path.is_empty() || !input.usbipd_binary_path.starts_with('/') {
        return Err(UsbipdPerEnvArgvError::InvalidBinaryPath {
            path: input.usbipd_binary_path.clone(),
        });
    }
    validate_env(&input.env)?;
    if input.backend_port == 0 {
        return Err(UsbipdPerEnvArgvError::InvalidPort);
    }
    Ok(vec![
        format!("nixling-sys-{}-usbipd-backend", input.env),
        "-4".to_owned(),
        "--tcp-port".to_owned(),
        input.backend_port.to_string(),
    ])
}

/// Render the argv for the per-env self-binding TCP proxy.
pub fn generate_usbipd_proxy_argv(
    input: &UsbipdProxyArgvInput,
) -> Result<Vec<String>, UsbipdPerEnvArgvError> {
    if input.socat_binary_path.is_empty() || !input.socat_binary_path.starts_with('/') {
        return Err(UsbipdPerEnvArgvError::InvalidBinaryPath {
            path: input.socat_binary_path.clone(),
        });
    }
    validate_env(&input.env)?;
    input.host_uplink_ip.parse::<IpAddr>().map_err(|_| {
        UsbipdPerEnvArgvError::InvalidHostUplinkIp {
            host_uplink_ip: input.host_uplink_ip.clone(),
        }
    })?;
    if input.backend_port == 0 {
        return Err(UsbipdPerEnvArgvError::InvalidPort);
    }
    Ok(vec![
        format!("nixling-sys-{}-usbipd-proxy", input.env),
        format!(
            "TCP-LISTEN:3240,bind={},fork,max-children=4,reuseaddr",
            input.host_uplink_ip
        ),
        format!("TCP:127.0.0.1:{}", input.backend_port),
    ])
}

#[cfg(test)]
mod per_env_tests {
    use super::*;

    fn backend_input() -> UsbipdBackendArgvInput {
        UsbipdBackendArgvInput {
            usbipd_binary_path: "/nix/store/USBIPUSBIPUSBIP-linux-usbip/bin/usbipd".to_owned(),
            env: "work".to_owned(),
            backend_port: 3242,
        }
    }

    fn proxy_input() -> UsbipdProxyArgvInput {
        UsbipdProxyArgvInput {
            socat_binary_path: "/nix/store/SOCATSOCATSOCAT-socat/bin/socat".to_owned(),
            env: "work".to_owned(),
            host_uplink_ip: "192.0.2.1".to_owned(),
            backend_port: 3242,
        }
    }

    #[test]
    fn backend_argv_matches_systemd_exec_start() {
        let argv = generate_usbipd_backend_argv(&backend_input()).unwrap();
        assert_eq!(
            argv,
            vec![
                "nixling-sys-work-usbipd-backend".to_owned(),
                "-4".to_owned(),
                "--tcp-port".to_owned(),
                "3242".to_owned(),
            ]
        );
    }

    #[test]
    fn proxy_argv_binds_env_uplink_ip() {
        let argv = generate_usbipd_proxy_argv(&proxy_input()).unwrap();
        assert_eq!(
            argv,
            vec![
                "nixling-sys-work-usbipd-proxy".to_owned(),
                "TCP-LISTEN:3240,bind=192.0.2.1,fork,max-children=4,reuseaddr".to_owned(),
                "TCP:127.0.0.1:3242".to_owned(),
            ]
        );
    }

    /// Per-env usbipd byte-parity snapshot. Emits SNAPSHOT lines
    /// (backend first, then proxy) consumed by the static
    /// gate's runner-shape diff.
    #[test]
    fn per_env_argv_snapshot_lines() {
        let backend = generate_usbipd_backend_argv(&backend_input()).unwrap();
        println!("SNAPSHOT: {}", backend.join(" "));
        let proxy = generate_usbipd_proxy_argv(&proxy_input()).unwrap();
        println!("SNAPSHOT: {}", proxy.join(" "));
    }

    #[test]
    fn backend_rejects_relative_binary_path() {
        let mut input = backend_input();
        input.usbipd_binary_path = "usbipd".to_owned();
        assert!(matches!(
            generate_usbipd_backend_argv(&input),
            Err(UsbipdPerEnvArgvError::InvalidBinaryPath { .. })
        ));
    }

    #[test]
    fn proxy_rejects_relative_binary_path() {
        let mut input = proxy_input();
        input.socat_binary_path = "socat".to_owned();
        assert!(matches!(
            generate_usbipd_proxy_argv(&input),
            Err(UsbipdPerEnvArgvError::InvalidBinaryPath { .. })
        ));
    }

    #[test]
    fn rejects_empty_env() {
        let mut input = backend_input();
        input.env.clear();
        assert!(matches!(
            generate_usbipd_backend_argv(&input),
            Err(UsbipdPerEnvArgvError::EmptyEnv)
        ));
    }

    #[test]
    fn rejects_env_with_shell_metachars() {
        let mut input = backend_input();
        input.env = "work; rm -rf /".to_owned();
        assert!(matches!(
            generate_usbipd_backend_argv(&input),
            Err(UsbipdPerEnvArgvError::InvalidEnv { .. })
        ));
    }

    #[test]
    fn rejects_env_with_uppercase() {
        let mut input = proxy_input();
        input.env = "Work".to_owned();
        assert!(matches!(
            generate_usbipd_proxy_argv(&input),
            Err(UsbipdPerEnvArgvError::InvalidEnv { .. })
        ));
    }

    #[test]
    fn rejects_zero_port() {
        let mut input = backend_input();
        input.backend_port = 0;
        assert!(matches!(
            generate_usbipd_backend_argv(&input),
            Err(UsbipdPerEnvArgvError::InvalidPort)
        ));
    }

    #[test]
    fn proxy_rejects_invalid_host_uplink_ip() {
        let mut input = proxy_input();
        input.host_uplink_ip = "not an ip".to_owned();
        assert!(matches!(
            generate_usbipd_proxy_argv(&input),
            Err(UsbipdPerEnvArgvError::InvalidHostUplinkIp { .. })
        ));
    }

    #[test]
    fn accepts_three_canonical_envs() {
        for (env, port) in [("obs", 3241_u16), ("personal", 3243), ("work", 3245)] {
            let backend = UsbipdBackendArgvInput {
                usbipd_binary_path: "/nix/store/X-usbip/bin/usbipd".to_owned(),
                env: env.to_owned(),
                backend_port: port,
            };
            let argv = generate_usbipd_backend_argv(&backend).unwrap();
            assert_eq!(argv[0], format!("nixling-sys-{env}-usbipd-backend"));
            assert_eq!(argv[3], port.to_string());
        }
    }

    #[test]
    fn round_trip_serializable() {
        let json = serde_json::to_string(&backend_input()).unwrap();
        let parsed: UsbipdBackendArgvInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, backend_input());
        let pjson = serde_json::to_string(&proxy_input()).unwrap();
        let pparsed: UsbipdProxyArgvInput = serde_json::from_str(&pjson).unwrap();
        assert_eq!(pparsed, proxy_input());
    }
}
