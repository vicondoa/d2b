//! Authenticated guest security-key frontend.

#![forbid(unsafe_code)]

pub mod framing;
pub mod services;
pub mod uhid;
pub mod vsock;

use std::{
    fmt,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use framing::{CtaphidGate, GateOutcome};
use services::security_key::{
    FrontendObservability, ReportStream, SessionConfig, TelemetryOutcome,
};
use uhid::{UhidDevice, UhidEvent};
use vsock::{BackoffParams, SK_VSOCK_PORT, connect_with_backoff};

#[derive(Clone)]
pub struct Config {
    vm_id: String,
    vsock_port: u32,
    uhid_path: PathBuf,
    session: SessionConfig,
}

impl fmt::Debug for Config {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Config")
            .field("vsock_port", &self.vsock_port)
            .field("uhid_path", &"<redacted>")
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

impl Config {
    pub fn from_env() -> Result<Self, &'static str> {
        let vm_id = std::env::var("D2B_SK_VM_ID").map_err(|_| "missing-vm-id")?;
        if !valid_vm_id(&vm_id) {
            return Err("invalid-vm-id");
        }
        let vsock_port = match std::env::var("D2B_SK_VSOCK_PORT") {
            Ok(value) => value.parse::<u32>().map_err(|_| "invalid-vsock-port")?,
            Err(_) => SK_VSOCK_PORT,
        };
        if vsock_port == 0 {
            return Err("invalid-vsock-port");
        }
        let uhid_path = std::env::var("D2B_SK_UHID_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/dev/uhid"));
        let session = SessionConfig::from_env()?;
        Ok(Self {
            vm_id,
            vsock_port,
            uhid_path,
            session,
        })
    }
}

fn valid_vm_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z'))
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

pub async fn run_from_env() -> Result<(), &'static str> {
    let config = Config::from_env()?;
    run(&config).await;
    Ok(())
}

pub async fn run(config: &Config) {
    let mut observations = FrontendObservability::default();
    loop {
        match UhidDevice::create(&config.uhid_path, &config.vm_id).await {
            Ok(mut device) => {
                fixed_log("uhid-ready");
                relay_loop(&mut device, config, &mut observations).await;
                fixed_log("uhid-recreate");
            }
            Err(_) => {
                observations.record(now_unix_ms(), TelemetryOutcome::Unavailable);
                fixed_log("uhid-unavailable");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn relay_loop(
    device: &mut UhidDevice,
    config: &Config,
    observations: &mut FrontendObservability,
) {
    let backoff = BackoffParams::default();
    loop {
        let transport = connect_with_backoff(config.vsock_port, backoff).await;
        let mut reports = match ReportStream::establish(transport, config.session.clone()).await {
            Ok(stream) => stream,
            Err(_) => {
                observations.record(now_unix_ms(), TelemetryOutcome::Unavailable);
                fixed_log("session-rejected");
                continue;
            }
        };
        fixed_log("session-ready");
        let outcome = run_relay(device, &mut reports, observations).await;
        let _ = reports.reset().await;
        match outcome {
            RelayOutcome::SessionDisconnected => fixed_log("session-disconnected"),
            RelayOutcome::UhidError => {
                fixed_log("uhid-error");
                return;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelayOutcome {
    SessionDisconnected,
    UhidError,
}

async fn run_relay(
    device: &mut UhidDevice,
    reports: &mut ReportStream,
    observations: &mut FrontendObservability,
) -> RelayOutcome {
    let mut gate = CtaphidGate::default();
    let mut keepalive = tokio::time::interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            remote = reports.receive_report() => {
                match remote {
                    Ok(report) => {
                        if device.send_input_report(&report).await.is_err() {
                            return RelayOutcome::UhidError;
                        }
                        observations.record(now_unix_ms(), TelemetryOutcome::Success);
                    }
                    Err(_) => return RelayOutcome::SessionDisconnected,
                }
            }
            local = device.read_event() => {
                match local {
                    Ok(Some(UhidEvent::Output { data, .. })) => {
                        match gate.accept(data) {
                            GateOutcome::Pending => {}
                            GateOutcome::Forward { reports: ready, .. } => {
                                for report in ready {
                                    if reports.send_report(&report).await.is_err() {
                                        return RelayOutcome::SessionDisconnected;
                                    }
                                }
                                observations.record(now_unix_ms(), TelemetryOutcome::Success);
                            }
                            GateOutcome::Denied { response } => {
                                if device.send_input_report(&response).await.is_err() {
                                    return RelayOutcome::UhidError;
                                }
                                observations.record(now_unix_ms(), TelemetryOutcome::Denied);
                            }
                        }
                    }
                    Ok(Some(UhidEvent::GetReport { id, .. })) => {
                        if device.send_get_report_reply_error(id).await.is_err() {
                            return RelayOutcome::UhidError;
                        }
                    }
                    Ok(Some(UhidEvent::Lifecycle(_))) => {}
                    Ok(Some(UhidEvent::Other(_))) => fixed_log("uhid-event-ignored"),
                    Ok(None) | Err(_) => return RelayOutcome::UhidError,
                }
            }
            _ = keepalive.tick() => {
                if reports.drive_keepalive().await.is_err() {
                    return RelayOutcome::SessionDisconnected;
                }
            }
        }
    }
}

fn fixed_log(event: &'static str) {
    eprintln!("[d2b-sk-frontend] {event}");
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_ids_use_the_framework_grammar() {
        for valid in ["a", "corp-vm", "vm2"] {
            assert!(valid_vm_id(valid));
        }
        for invalid in ["", "2vm", "Corp", "vm_name", "vm/one"] {
            assert!(!valid_vm_id(invalid));
        }
    }

    #[test]
    fn config_debug_redacts_identity_and_path() {
        let config = Config {
            vm_id: "corp-vm".into(),
            vsock_port: 14320,
            uhid_path: "/sensitive/device".into(),
            session: SessionConfig::new([7; 32], 1).unwrap(),
        };
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("corp-vm"));
        assert!(!rendered.contains("/sensitive/device"));
        assert!(!rendered.contains(&"07".repeat(32)));
    }
}
