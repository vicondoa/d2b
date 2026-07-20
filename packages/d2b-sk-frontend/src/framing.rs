//! Bounded CTAPHID request assembly and fail-closed command classification.

use std::collections::BTreeMap;

use d2b_provider_device_host_mediated::{
    FidoCeremonyApproval, FidoCommandKind, FidoPolicyDecision, FidoPolicyIntent,
};

pub const CTAPHID_REPORT_LEN: usize = 64;
pub const MAX_CTAPHID_MESSAGE_BYTES: usize = 7_609;
pub const MAX_IN_FLIGHT_CHANNELS: usize = 8;

const CTAPHID_PING: u8 = 0x81;
const CTAPHID_INIT: u8 = 0x86;
const CTAPHID_WINK: u8 = 0x88;
const CTAPHID_CBOR: u8 = 0x90;
const CTAPHID_CANCEL: u8 = 0x91;
const CTAPHID_ERROR: u8 = 0xbf;
const CTAPHID_ERR_INVALID_CMD: u8 = 0x01;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalRequirement {
    None,
    TrustedController,
}

#[derive(Debug, PartialEq, Eq)]
pub enum GateOutcome {
    Pending,
    Forward {
        reports: Vec<[u8; CTAPHID_REPORT_LEN]>,
        approval: ApprovalRequirement,
    },
    Denied {
        response: [u8; CTAPHID_REPORT_LEN],
    },
}

struct PendingCommand {
    command: u8,
    expected_bytes: usize,
    next_sequence: u8,
    payload: Vec<u8>,
    reports: Vec<[u8; CTAPHID_REPORT_LEN]>,
}

#[derive(Default)]
pub struct CtaphidGate {
    pending: BTreeMap<[u8; 4], PendingCommand>,
}

impl CtaphidGate {
    pub fn accept(&mut self, report: [u8; CTAPHID_REPORT_LEN]) -> GateOutcome {
        let channel = [report[0], report[1], report[2], report[3]];
        if report[4] & 0x80 != 0 {
            return self.begin(channel, report);
        }
        self.continue_command(channel, report)
    }

    fn begin(&mut self, channel: [u8; 4], report: [u8; CTAPHID_REPORT_LEN]) -> GateOutcome {
        let expected_bytes = usize::from(u16::from_be_bytes([report[5], report[6]]));
        if expected_bytes > MAX_CTAPHID_MESSAGE_BYTES
            || self.pending.contains_key(&channel)
            || (self.pending.len() >= MAX_IN_FLIGHT_CHANNELS && expected_bytes > 57)
        {
            self.pending.remove(&channel);
            return denied(channel);
        }

        let available = expected_bytes.min(57);
        let mut pending = PendingCommand {
            command: report[4],
            expected_bytes,
            next_sequence: 0,
            payload: report[7..7 + available].to_vec(),
            reports: vec![report],
        };
        if pending.payload.len() == pending.expected_bytes {
            return classify(pending);
        }
        pending
            .payload
            .reserve(pending.expected_bytes.saturating_sub(pending.payload.len()));
        self.pending.insert(channel, pending);
        GateOutcome::Pending
    }

    fn continue_command(
        &mut self,
        channel: [u8; 4],
        report: [u8; CTAPHID_REPORT_LEN],
    ) -> GateOutcome {
        let Some(mut pending) = self.pending.remove(&channel) else {
            return denied(channel);
        };
        if report[4] != pending.next_sequence {
            return denied(channel);
        }
        pending.next_sequence = match pending.next_sequence.checked_add(1) {
            Some(next) if next <= 0x80 => next,
            _ => return denied(channel),
        };
        let remaining = pending.expected_bytes.saturating_sub(pending.payload.len());
        let copied = remaining.min(59);
        pending.payload.extend_from_slice(&report[5..5 + copied]);
        pending.reports.push(report);
        if pending.reports.len() > 129 {
            return denied(channel);
        }
        if pending.payload.len() == pending.expected_bytes {
            classify(pending)
        } else {
            self.pending.insert(channel, pending);
            GateOutcome::Pending
        }
    }
}

fn classify(pending: PendingCommand) -> GateOutcome {
    let decision = match pending.command {
        CTAPHID_INIT | CTAPHID_PING | CTAPHID_WINK | CTAPHID_CANCEL => {
            return GateOutcome::Forward {
                reports: pending.reports,
                approval: ApprovalRequirement::None,
            };
        }
        CTAPHID_CBOR => classify_cbor(&pending.payload),
        _ => FidoPolicyDecision::DenyDestructive,
    };
    match decision {
        FidoPolicyDecision::AllowReadOnly => GateOutcome::Forward {
            reports: pending.reports,
            approval: ApprovalRequirement::None,
        },
        FidoPolicyDecision::DenyApprovalRequired => GateOutcome::Forward {
            reports: pending.reports,
            approval: ApprovalRequirement::TrustedController,
        },
        FidoPolicyDecision::AllowApprovedCeremony => {
            unreachable!("the frontend never originates trusted approval")
        }
        FidoPolicyDecision::DenyDestructive => {
            let report = pending.reports[0];
            denied([report[0], report[1], report[2], report[3]])
        }
    }
}

fn classify_cbor(payload: &[u8]) -> FidoPolicyDecision {
    let Some((&command, request)) = payload.split_first() else {
        return FidoPolicyDecision::DenyDestructive;
    };
    let command = match command {
        0x01 => FidoCommandKind::MakeCredential,
        0x02 => FidoCommandKind::GetAssertion,
        0x04 => FidoCommandKind::GetInfo,
        0x06 => {
            return FidoPolicyIntent::canonical()
                .decide_client_pin(request, FidoCeremonyApproval::Missing);
        }
        0x07 => FidoCommandKind::Reset,
        0x08 => FidoCommandKind::GetNextAssertion,
        0x09 => FidoCommandKind::BioEnrollment,
        0x0a => FidoCommandKind::CredentialManagement,
        0x0b => FidoCommandKind::Selection,
        0x0c => FidoCommandKind::LargeBlobs,
        0x0d => FidoCommandKind::AuthenticatorConfiguration,
        0x40..=0xff => FidoCommandKind::Vendor,
        _ => FidoCommandKind::Unknown,
    };
    FidoPolicyIntent::canonical().decide(command, FidoCeremonyApproval::Missing)
}

fn denied(channel: [u8; 4]) -> GateOutcome {
    let mut response = [0; CTAPHID_REPORT_LEN];
    response[..4].copy_from_slice(&channel);
    response[4] = CTAPHID_ERROR;
    response[6] = 1;
    response[7] = CTAPHID_ERR_INVALID_CMD;
    GateOutcome::Denied { response }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initial(channel: u32, command: u8, payload: &[u8]) -> [u8; CTAPHID_REPORT_LEN] {
        let mut report = [0; CTAPHID_REPORT_LEN];
        report[..4].copy_from_slice(&channel.to_be_bytes());
        report[4] = command;
        report[5..7].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        report[7..7 + payload.len().min(57)].copy_from_slice(&payload[..payload.len().min(57)]);
        report
    }

    fn continuation(channel: u32, sequence: u8, payload: &[u8]) -> [u8; CTAPHID_REPORT_LEN] {
        let mut report = [0; CTAPHID_REPORT_LEN];
        report[..4].copy_from_slice(&channel.to_be_bytes());
        report[4] = sequence;
        report[5..5 + payload.len().min(59)].copy_from_slice(&payload[..payload.len().min(59)]);
        report
    }

    #[test]
    fn read_only_get_info_forwards_without_approval() {
        let outcome = CtaphidGate::default().accept(initial(1, CTAPHID_CBOR, &[0x04]));
        assert!(matches!(
            outcome,
            GateOutcome::Forward {
                approval: ApprovalRequirement::None,
                ..
            }
        ));
    }

    #[test]
    fn ceremony_requires_the_authenticated_controller() {
        let outcome = CtaphidGate::default().accept(initial(1, CTAPHID_CBOR, &[0x02]));
        assert!(matches!(
            outcome,
            GateOutcome::Forward {
                approval: ApprovalRequirement::TrustedController,
                ..
            }
        ));
    }

    #[test]
    fn destructive_and_unknown_commands_fail_closed() {
        for payload in [&[0x07][..], &[0x0a][..], &[0x40][..], &[0x7f][..]] {
            assert!(matches!(
                CtaphidGate::default().accept(initial(9, CTAPHID_CBOR, payload)),
                GateOutcome::Denied { .. }
            ));
        }
        assert!(matches!(
            CtaphidGate::default().accept(initial(9, 0x83, &[])),
            GateOutcome::Denied { .. }
        ));
    }

    #[test]
    fn client_pin_read_only_subset_is_closed() {
        let get_retries = [0x06, 0xa1, 0x02, 0x01];
        let set_pin = [0x06, 0xa1, 0x02, 0x03];
        assert!(matches!(
            CtaphidGate::default().accept(initial(2, CTAPHID_CBOR, &get_retries)),
            GateOutcome::Forward {
                approval: ApprovalRequirement::None,
                ..
            }
        ));
        assert!(matches!(
            CtaphidGate::default().accept(initial(2, CTAPHID_CBOR, &set_pin)),
            GateOutcome::Denied { .. }
        ));
    }

    #[test]
    fn complete_message_is_buffered_before_forwarding() {
        let payload = vec![0x55; 80];
        let mut gate = CtaphidGate::default();
        assert_eq!(
            gate.accept(initial(3, CTAPHID_PING, &payload)),
            GateOutcome::Pending
        );
        match gate.accept(continuation(3, 0, &payload[57..])) {
            GateOutcome::Forward { reports, .. } => assert_eq!(reports.len(), 2),
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[test]
    fn bad_sequence_and_oversize_are_denied() {
        let payload = vec![0; 80];
        let mut gate = CtaphidGate::default();
        assert_eq!(
            gate.accept(initial(4, CTAPHID_PING, &payload)),
            GateOutcome::Pending
        );
        assert!(matches!(
            gate.accept(continuation(4, 1, &payload[57..])),
            GateOutcome::Denied { .. }
        ));

        let mut oversize = initial(5, CTAPHID_PING, &[]);
        oversize[5..7].copy_from_slice(&((MAX_CTAPHID_MESSAGE_BYTES + 1) as u16).to_be_bytes());
        assert!(matches!(
            CtaphidGate::default().accept(oversize),
            GateOutcome::Denied { .. }
        ));
    }

    #[test]
    fn in_flight_channels_are_bounded() {
        let payload = vec![0; 80];
        let mut gate = CtaphidGate::default();
        for channel in 1..=MAX_IN_FLIGHT_CHANNELS as u32 {
            assert_eq!(
                gate.accept(initial(channel, CTAPHID_PING, &payload)),
                GateOutcome::Pending
            );
        }
        assert!(matches!(
            gate.accept(initial(99, CTAPHID_PING, &payload)),
            GateOutcome::Denied { .. }
        ));
    }
}
