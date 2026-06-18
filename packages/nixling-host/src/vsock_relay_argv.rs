//! vsock relay argv generator.
//!
//! Pure Rust function emitting the socat argv for the per-VM vsock relay
//! sidecars used by `nixling.observability.*`. The observability work
//! shipped three shapes documented in
//! `nixos-modules/components/observability/{host,guest,stack}.nix`:
//!
//! - **stack-vm vsock-in**: `socat VSOCK-LISTEN:14317,fork,...
//!   UNIX-CONNECT:/run/nixling/obs-ingress.sock` — the obs stack VM
//!   listens on a vsock port and forwards to an in-guest Alloy.
//! - **guest egress**: `socat UNIX-LISTEN:<sock>,fork,...
//!   VSOCK-CONNECT:2:14317` — the guest Alloy egresses to host
//!   CID 2 (the daemon-host) on port 14317.
//! - **host bridge** (EXEC form): `socat UNIX-LISTEN:<sock>,fork,...
//!   EXEC:"<chVsockConnect> <base> 14317"` — used when CH only
//!   exposes the GUEST→HOST direction on demand.
//!
//! This generator covers the LISTEN+CONNECT shapes (the EXEC form
//! is a more specialized helper that lives in
//! `nixos-modules/components/observability/host.nix` and is not
//! generalizable to a generic argv generator without the
//! ch-vsock-connect package path).
//!
//! Crate invariant `#![forbid(unsafe_code)]` is honoured.

use serde::{Deserialize, Serialize};

/// One side of a socat pipe. The generator stitches two
/// [`SocatEndpoint`] entries together with the documented socat
/// option set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum SocatEndpoint {
    /// `UNIX-LISTEN:<path>` with the standard fork/reuseaddr/mode
    /// options the observability harness uses.
    UnixListen {
        path: String,
        /// `max-children=N` cap; obs harness uses 16 for the guest
        /// egress, omits for host-bridge (single-writer).
        max_children: Option<u32>,
        /// `mode=0NNN` octal. Obs harness uses `0660` for both
        /// directions so the daemon-owned group can connect.
        mode: u32,
    },
    /// `UNIX-CONNECT:<path>`. No options.
    UnixConnect { path: String },
    /// `VSOCK-LISTEN:<port>` with fork/reuseaddr. obs stack VM uses
    /// max-children=16 on port 14317.
    VsockListen {
        port: u32,
        max_children: Option<u32>,
    },
    /// `VSOCK-CONNECT:<cid>:<port>`. Guest egress uses CID 2 (host)
    /// port 14317.
    VsockConnect { cid: u32, port: u32 },
}

impl SocatEndpoint {
    pub fn render(&self) -> String {
        match self {
            Self::UnixListen {
                path,
                max_children,
                mode,
            } => {
                let mut parts = vec![
                    format!("UNIX-LISTEN:{path}"),
                    "fork".to_owned(),
                    "reuseaddr".to_owned(),
                ];
                if let Some(mc) = max_children {
                    parts.push(format!("max-children={mc}"));
                }
                parts.push(format!("mode=0{mode:o}"));
                parts.join(",")
            }
            Self::UnixConnect { path } => format!("UNIX-CONNECT:{path}"),
            Self::VsockListen { port, max_children } => {
                let mut parts = vec![
                    format!("VSOCK-LISTEN:{port}"),
                    "fork".to_owned(),
                    "reuseaddr".to_owned(),
                ];
                if let Some(mc) = max_children {
                    parts.push(format!("max-children={mc}"));
                }
                parts.join(",")
            }
            Self::VsockConnect { cid, port } => format!("VSOCK-CONNECT:{cid}:{port}"),
        }
    }
}

/// All inputs required to render the vsock-relay socat argv.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VsockRelayArgvInput {
    /// Absolute store path to the `socat` binary.
    pub socat_binary_path: String,
    /// VM / role context for [`exec_arg0`]. The observability harness
    /// uses `nixling-otel-relay@<vm>` for guest→host relays and
    /// `nixling-otel-vsock-in` for the obs stack VM listener.
    pub relay_name: String,
    /// Left side of the pipe (the LISTEN side). The generator emits
    /// `-d -d` (debug) before the endpoints, matching the observability
    /// harness's ExecStart shape.
    pub source: SocatEndpoint,
    /// Right side of the pipe (the CONNECT side).
    pub sink: SocatEndpoint,
    /// Free-form additional socat args appended at the end.
    #[serde(default)]
    pub extra_args: Vec<String>,
}

/// Errors the vsock-relay argv generator can return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum VsockRelayArgvError {
    InvalidSocatBinaryPath {
        path: String,
    },
    EmptyRelayName,
    /// Both endpoints must contain a non-empty target.
    EmptyEndpoint {
        which: String,
    },
    /// The source MUST be a LISTEN side (one of UnixListen / VsockListen).
    /// Two CONNECTs back-to-back would require socat to run both as clients
    /// — that's a different shape (proxy chain) the harness does not support.
    SourceMustBeListen,
    /// socat's address syntax is `<type>:<address-data>[,option[,option...]]`.
    /// A UNIX path
    /// containing a comma (or any other socat option-syntax
    /// character) injects arbitrary socat options when interpolated
    /// into `UNIX-LISTEN:<path>,fork,...`. The fix refuses any path
    /// containing `,`, `!`, `"`, `'`, `;`, or whitespace — those
    /// have no legitimate use in a UDS path under
    /// `/run/nixling/...` or `/var/lib/nixling/...` and a bundle row
    /// supplying one is unambiguously hostile.
    PathContainsSocatMetachar {
        path: String,
        character: char,
    },
}

/// Validate a UDS path against socat option-syntax injection. The
/// observability harness path namespace (`/run/nixling/...`,
/// `/var/lib/nixling/...`) only ever contains ASCII letters / digits /
/// `_-./`; refusing
/// anything else closes the historic denylist gap (the v1 denylist
/// missed `:` which socat treats as an address-parameter separator,
/// plus `[]{}()` and various control chars). Switched to an
/// allowlist so future socat option additions can't widen the
/// attack surface.
fn socat_unsafe_metachar(s: &str) -> Option<char> {
    // ALLOWLIST: ASCII alphanumeric, plus `_`, `-`, `.`, `/`. Anything
    // else (commas, colons, brackets, whitespace, NUL, etc.) is
    // refused as an unsafe path character.
    s.chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/')))
}

fn endpoint_kind(e: &SocatEndpoint) -> &'static str {
    match e {
        SocatEndpoint::UnixListen { .. } => "unix-listen",
        SocatEndpoint::UnixConnect { .. } => "unix-connect",
        SocatEndpoint::VsockListen { .. } => "vsock-listen",
        SocatEndpoint::VsockConnect { .. } => "vsock-connect",
    }
}

fn endpoint_non_empty(e: &SocatEndpoint, which: &str) -> Result<(), VsockRelayArgvError> {
    match e {
        SocatEndpoint::UnixListen { path, .. } | SocatEndpoint::UnixConnect { path } => {
            if path.is_empty() {
                return Err(VsockRelayArgvError::EmptyEndpoint {
                    which: which.to_owned(),
                });
            }
            // Refuse socat address-syntax metacharacters in the path.
            // See
            // VsockRelayArgvError::PathContainsSocatMetachar.
            if let Some(c) = socat_unsafe_metachar(path) {
                return Err(VsockRelayArgvError::PathContainsSocatMetachar {
                    path: path.clone(),
                    character: c,
                });
            }
        }
        SocatEndpoint::VsockListen { .. } | SocatEndpoint::VsockConnect { .. } => {
            // VsockListen/Connect carry numeric inputs which serde
            // already validates as non-negative; no further check
            // needed at this layer.
        }
    }
    Ok(())
}

/// Render the socat argv.
pub fn generate_vsock_relay_argv(
    input: &VsockRelayArgvInput,
) -> Result<Vec<String>, VsockRelayArgvError> {
    if input.socat_binary_path.is_empty() || !input.socat_binary_path.starts_with('/') {
        return Err(VsockRelayArgvError::InvalidSocatBinaryPath {
            path: input.socat_binary_path.clone(),
        });
    }
    if input.relay_name.is_empty() {
        return Err(VsockRelayArgvError::EmptyRelayName);
    }

    if !matches!(endpoint_kind(&input.source), "unix-listen" | "vsock-listen") {
        return Err(VsockRelayArgvError::SourceMustBeListen);
    }
    endpoint_non_empty(&input.source, "source")?;
    endpoint_non_empty(&input.sink, "sink")?;

    let mut argv: Vec<String> = vec![
        input.socat_binary_path.clone(),
        // The observability harness uses `-d -d` (double-debug) so the
        // relay logs each connection. Pinned here so generator
        // output diff'd against the harness ExecStart stays stable.
        "-d".to_owned(),
        "-d".to_owned(),
        input.source.render(),
        input.sink.render(),
    ];
    for extra in &input.extra_args {
        argv.push(extra.clone());
    }
    Ok(argv)
}

/// `arg0` for the relay process.
pub fn exec_arg0(input: &VsockRelayArgvInput) -> Result<String, VsockRelayArgvError> {
    if input.relay_name.is_empty() {
        return Err(VsockRelayArgvError::EmptyRelayName);
    }
    Ok(input.relay_name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Observability harness stack-vm vsock-in shape:
    /// `socat -d -d VSOCK-LISTEN:14317,fork,max-children=16,reuseaddr UNIX-CONNECT:/run/nixling/obs-ingress.sock`
    fn audit_stack_vsock_in() -> VsockRelayArgvInput {
        VsockRelayArgvInput {
            socat_binary_path: "/nix/store/SOCATSOCATSOCATSOCATSOCAT-socat/bin/socat".to_owned(),
            relay_name: "nixling-otel-vsock-in".to_owned(),
            source: SocatEndpoint::VsockListen {
                port: 14317,
                max_children: Some(16),
            },
            sink: SocatEndpoint::UnixConnect {
                path: "/run/nixling/obs-ingress.sock".to_owned(),
            },
            extra_args: Vec::new(),
        }
    }

    /// Observability harness guest egress shape:
    /// `socat -d -d UNIX-LISTEN:<sock>,fork,max-children=16,reuseaddr,mode=0660 VSOCK-CONNECT:2:14317`
    fn audit_guest_egress() -> VsockRelayArgvInput {
        VsockRelayArgvInput {
            socat_binary_path: "/nix/store/SOCATSOCATSOCATSOCATSOCAT-socat/bin/socat".to_owned(),
            relay_name: "nixling-otel-egress@corp-vm".to_owned(),
            source: SocatEndpoint::UnixListen {
                path: "/run/nixling/otlp.sock".to_owned(),
                max_children: Some(16),
                mode: 0o660,
            },
            sink: SocatEndpoint::VsockConnect {
                cid: 2,
                port: 14317,
            },
            extra_args: Vec::new(),
        }
    }

    #[test]
    fn stack_vsock_in_parity() {
        let argv = generate_vsock_relay_argv(&audit_stack_vsock_in()).unwrap();
        assert!(argv[0].ends_with("/socat"));
        assert_eq!(argv[1], "-d");
        assert_eq!(argv[2], "-d");
        assert_eq!(argv[3], "VSOCK-LISTEN:14317,fork,reuseaddr,max-children=16");
        assert_eq!(argv[4], "UNIX-CONNECT:/run/nixling/obs-ingress.sock");
    }

    #[test]
    fn guest_egress_parity() {
        let argv = generate_vsock_relay_argv(&audit_guest_egress()).unwrap();
        assert_eq!(
            argv[3],
            "UNIX-LISTEN:/run/nixling/otlp.sock,fork,reuseaddr,max-children=16,mode=0660"
        );
        assert_eq!(argv[4], "VSOCK-CONNECT:2:14317");
    }

    #[test]
    fn exec_arg0_round_trip() {
        assert_eq!(
            exec_arg0(&audit_stack_vsock_in()).unwrap(),
            "nixling-otel-vsock-in"
        );
        assert_eq!(
            exec_arg0(&audit_guest_egress()).unwrap(),
            "nixling-otel-egress@corp-vm"
        );
    }

    #[test]
    fn rejects_non_absolute_socat() {
        let mut input = audit_stack_vsock_in();
        input.socat_binary_path = "socat".to_owned();
        assert!(matches!(
            generate_vsock_relay_argv(&input),
            Err(VsockRelayArgvError::InvalidSocatBinaryPath { .. })
        ));
    }

    #[test]
    fn rejects_empty_socat() {
        let mut input = audit_stack_vsock_in();
        input.socat_binary_path.clear();
        assert!(matches!(
            generate_vsock_relay_argv(&input),
            Err(VsockRelayArgvError::InvalidSocatBinaryPath { .. })
        ));
    }

    #[test]
    fn rejects_empty_relay_name() {
        let mut input = audit_stack_vsock_in();
        input.relay_name.clear();
        assert!(matches!(
            generate_vsock_relay_argv(&input),
            Err(VsockRelayArgvError::EmptyRelayName)
        ));
    }

    #[test]
    fn rejects_source_that_is_connect() {
        // UNIX-CONNECT on the source side is invalid (two clients).
        let mut input = audit_stack_vsock_in();
        input.source = SocatEndpoint::UnixConnect {
            path: "/tmp/foo.sock".to_owned(),
        };
        assert!(matches!(
            generate_vsock_relay_argv(&input),
            Err(VsockRelayArgvError::SourceMustBeListen)
        ));
    }

    #[test]
    fn rejects_source_vsock_connect() {
        let mut input = audit_stack_vsock_in();
        input.source = SocatEndpoint::VsockConnect {
            cid: 2,
            port: 14317,
        };
        assert!(matches!(
            generate_vsock_relay_argv(&input),
            Err(VsockRelayArgvError::SourceMustBeListen)
        ));
    }

    #[test]
    fn rejects_empty_endpoint_path_in_source() {
        let mut input = audit_stack_vsock_in();
        input.source = SocatEndpoint::UnixListen {
            path: String::new(),
            max_children: None,
            mode: 0o660,
        };
        assert!(matches!(
            generate_vsock_relay_argv(&input),
            Err(VsockRelayArgvError::EmptyEndpoint { .. })
        ));
    }

    #[test]
    fn rejects_empty_endpoint_path_in_sink() {
        let mut input = audit_stack_vsock_in();
        input.sink = SocatEndpoint::UnixConnect {
            path: String::new(),
        };
        assert!(matches!(
            generate_vsock_relay_argv(&input),
            Err(VsockRelayArgvError::EmptyEndpoint { .. })
        ));
    }

    #[test]
    fn omits_max_children_when_absent() {
        let mut input = audit_stack_vsock_in();
        input.source = SocatEndpoint::VsockListen {
            port: 14317,
            max_children: None,
        };
        let argv = generate_vsock_relay_argv(&input).unwrap();
        assert!(!argv[3].contains("max-children"));
    }

    #[test]
    fn unix_listen_renders_mode_octal() {
        let e = SocatEndpoint::UnixListen {
            path: "/run/x.sock".to_owned(),
            max_children: None,
            mode: 0o644,
        };
        assert_eq!(
            e.render(),
            "UNIX-LISTEN:/run/x.sock,fork,reuseaddr,mode=0644"
        );
    }

    #[test]
    fn extra_args_appended_in_order() {
        let mut input = audit_stack_vsock_in();
        input.extra_args = vec!["-v".to_owned()];
        let argv = generate_vsock_relay_argv(&input).unwrap();
        assert_eq!(argv.last().unwrap(), "-v");
    }

    #[test]
    fn argv_is_round_trip_serializable() {
        let input = audit_guest_egress();
        let json = serde_json::to_string(&input).unwrap();
        let parsed: VsockRelayArgvInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, input);
    }

    /// Refuse socat path injection attempts via comma / quote /
    /// semicolon / whitespace in UDS paths.
    #[test]
    fn rejects_unix_listen_path_with_comma_injection() {
        let mut input = audit_guest_egress();
        input.source = SocatEndpoint::UnixListen {
            path: "/tmp/x,exec:/bin/sh".to_owned(),
            max_children: Some(16),
            mode: 0o660,
        };
        assert!(matches!(
            generate_vsock_relay_argv(&input),
            Err(VsockRelayArgvError::PathContainsSocatMetachar { character: ',', .. })
        ));
    }

    #[test]
    fn rejects_unix_connect_path_with_semicolon_injection() {
        let mut input = audit_stack_vsock_in();
        input.sink = SocatEndpoint::UnixConnect {
            path: "/run/x.sock;rm -rf /".to_owned(),
        };
        assert!(matches!(
            generate_vsock_relay_argv(&input),
            Err(VsockRelayArgvError::PathContainsSocatMetachar { character: ';', .. })
        ));
    }

    #[test]
    fn rejects_path_with_whitespace() {
        let mut input = audit_stack_vsock_in();
        input.sink = SocatEndpoint::UnixConnect {
            path: "/run/has space.sock".to_owned(),
        };
        assert!(matches!(
            generate_vsock_relay_argv(&input),
            Err(VsockRelayArgvError::PathContainsSocatMetachar { character: ' ', .. })
        ));
    }

    #[test]
    fn rejects_path_with_quote() {
        let mut input = audit_stack_vsock_in();
        input.sink = SocatEndpoint::UnixConnect {
            path: "/tmp/\"injection\".sock".to_owned(),
        };
        assert!(matches!(
            generate_vsock_relay_argv(&input),
            Err(VsockRelayArgvError::PathContainsSocatMetachar { character: '"', .. })
        ));
    }

    /// Explicit test for UnixListen → UnixConnect shape (the production
    /// observability host-bridge path uses it).
    #[test]
    fn unix_listen_to_unix_connect_shape_is_supported() {
        let input = VsockRelayArgvInput {
            socat_binary_path: "/nix/store/SOCATSOCATSOCATSOCATSOCAT-socat/bin/socat".to_owned(),
            relay_name: "nixling-host-bridge".to_owned(),
            source: SocatEndpoint::UnixListen {
                path: "/run/nixling/host-egress.sock".to_owned(),
                max_children: None,
                mode: 0o660,
            },
            sink: SocatEndpoint::UnixConnect {
                path: "/run/nixling/forward.sock".to_owned(),
            },
            extra_args: Vec::new(),
        };
        let argv = generate_vsock_relay_argv(&input).unwrap();
        assert!(argv[3].starts_with("UNIX-LISTEN:"));
        assert!(argv[4].starts_with("UNIX-CONNECT:"));
    }

    /// socat treats `:` as an address-parameter separator. The previous
    /// denylist missed it; the fix switched to an allowlist
    /// `[A-Za-z0-9_./-]` which closes `:`, brackets, and any other socat
    /// option-syntax character.
    #[test]
    fn rejects_path_with_colon() {
        let mut input = audit_stack_vsock_in();
        input.sink = SocatEndpoint::UnixConnect {
            path: "/run/nixling/a:b".to_owned(),
        };
        assert!(matches!(
            generate_vsock_relay_argv(&input),
            Err(VsockRelayArgvError::PathContainsSocatMetachar { character: ':', .. })
        ));
    }

    #[test]
    fn rejects_path_with_brackets() {
        let mut input = audit_stack_vsock_in();
        input.sink = SocatEndpoint::UnixConnect {
            path: "/run/nixling/[evil].sock".to_owned(),
        };
        assert!(matches!(
            generate_vsock_relay_argv(&input),
            Err(VsockRelayArgvError::PathContainsSocatMetachar { .. })
        ));
    }

    /// Vsock-relay golden parity: argv.join(" ") byte-equals the
    /// matching line in tests/golden/runner-shape/vsock-relay-argv-minimal.txt.
    /// Two shapes covered (EXEC form is OtelHostBridge's golden):
    ///   line 0 -> stack-vm vsock-in fixture (audit_stack_vsock_in)
    ///   line 1 -> guest egress fixture       (audit_guest_egress)
    const VSOCK_RELAY_GOLDEN: &str =
        include_str!("../../../tests/golden/runner-shape/vsock-relay-argv-minimal.txt");

    fn golden_payload_lines() -> Vec<&'static str> {
        VSOCK_RELAY_GOLDEN
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect()
    }

    #[test]
    fn golden_has_exactly_two_shape_lines() {
        let lines = golden_payload_lines();
        assert_eq!(
            lines.len(),
            2,
            "golden file must carry exactly two argv shapes (stack vsock-in + guest egress); EXEC form lives in OtelHostBridge's golden"
        );
    }

    #[test]
    fn golden_parity_stack_vsock_in() {
        let argv = generate_vsock_relay_argv(&audit_stack_vsock_in()).unwrap();
        let actual = argv.join(" ");
        let expected = golden_payload_lines()[0];
        assert_eq!(
            actual, expected,
            "stack-vm vsock-in argv drifted from tests/golden/runner-shape/vsock-relay-argv-minimal.txt"
        );
    }

    #[test]
    fn golden_parity_guest_egress() {
        let argv = generate_vsock_relay_argv(&audit_guest_egress()).unwrap();
        let actual = argv.join(" ");
        let expected = golden_payload_lines()[1];
        assert_eq!(
            actual, expected,
            "guest-egress argv drifted from tests/golden/runner-shape/vsock-relay-argv-minimal.txt"
        );
    }

    /// SNAPSHOT printers kept for ad-hoc diffs; golden parity is asserted
    /// by the unit tests above.
    #[test]
    fn stack_vsock_in_snapshot_line() {
        let argv = generate_vsock_relay_argv(&audit_stack_vsock_in()).unwrap();
        println!("SNAPSHOT: {}", argv.join(" "));
    }

    #[test]
    fn guest_egress_snapshot_line() {
        let argv = generate_vsock_relay_argv(&audit_guest_egress()).unwrap();
        println!("SNAPSHOT: {}", argv.join(" "));
    }

    #[test]
    fn rejects_path_with_nul() {
        let mut input = audit_stack_vsock_in();
        input.sink = SocatEndpoint::UnixConnect {
            path: "/run/nixling/\0evil.sock".to_owned(),
        };
        assert!(matches!(
            generate_vsock_relay_argv(&input),
            Err(VsockRelayArgvError::PathContainsSocatMetachar { .. })
        ));
    }
}
