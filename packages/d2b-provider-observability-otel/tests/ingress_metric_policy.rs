use std::collections::BTreeMap;

use d2b_provider_observability_otel::ingress_policy::QUARANTINE_VIOLATION_THRESHOLD;
use d2b_provider_observability_otel::{
    IdentityCanaries, Ingress, IngressErrorClass, IngressOutcome, IngressPolicyGate,
    MetricDescriptor, MetricFrame, MetricPoint, canonical_descriptor, label,
};

const ALL_INGRESSES: [Ingress; 4] = [
    Ingress::EmitterUnix,
    Ingress::OtlpUnix,
    Ingress::OtlpVsock,
    Ingress::ImportStream,
];

#[derive(Clone, Copy)]
enum PolicyCase {
    Oversize,
    MalformedResourceAttributes,
    EmptyPointSet,
    KeyNotAllowlisted,
    KeyForbidden,
    KeySuffixForbidden,
    ValueIdentity,
}

impl PolicyCase {
    const ALL: [Self; 7] = [
        Self::Oversize,
        Self::MalformedResourceAttributes,
        Self::EmptyPointSet,
        Self::KeyNotAllowlisted,
        Self::KeyForbidden,
        Self::KeySuffixForbidden,
        Self::ValueIdentity,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Oversize => "oversize",
            Self::MalformedResourceAttributes => "malformed resource attributes",
            Self::EmptyPointSet => "empty point set",
            Self::KeyNotAllowlisted => "key not allowlisted",
            Self::KeyForbidden => "key forbidden",
            Self::KeySuffixForbidden => "key suffix forbidden",
            Self::ValueIdentity => "value identity",
        }
    }

    fn input(self) -> (MetricFrame, IdentityCanaries, IngressErrorClass) {
        match self {
            Self::Oversize => (
                MetricFrame::new(
                    0,
                    std::iter::repeat_with(oversize_point).take(1024),
                    valid_resource_attributes(),
                ),
                IdentityCanaries::default(),
                IngressErrorClass::Oversize,
            ),
            Self::MalformedResourceAttributes => (
                MetricFrame::new(
                    64,
                    [valid_point()],
                    BTreeMap::from([("d2b.zone".to_owned(), "work/".to_owned())]),
                ),
                IdentityCanaries::default(),
                IngressErrorClass::Malformed,
            ),
            Self::EmptyPointSet => (
                MetricFrame::new(64, Vec::<MetricPoint>::new(), valid_resource_attributes()),
                IdentityCanaries::default(),
                IngressErrorClass::Malformed,
            ),
            Self::KeyNotAllowlisted => (
                frame_with_label("unknown", &["work"], "work"),
                IdentityCanaries::default(),
                IngressErrorClass::KeyNotAllowlisted,
            ),
            Self::KeyForbidden => (
                frame_with_label("vm", &["work"], "work"),
                IdentityCanaries::default(),
                IngressErrorClass::KeyForbidden,
            ),
            Self::KeySuffixForbidden => (
                frame_with_label("resource_name", &["work"], "work"),
                IdentityCanaries::default(),
                IngressErrorClass::KeySuffixForbidden,
            ),
            Self::ValueIdentity => (
                valid_frame(),
                IdentityCanaries::new(
                    ["accepted"],
                    std::iter::empty::<&str>(),
                    std::iter::empty::<&str>(),
                ),
                IngressErrorClass::ValueIdentity,
            ),
        }
    }
}

fn valid_resource_attributes() -> BTreeMap<String, String> {
    BTreeMap::from([(
        "d2b.zone".to_owned(),
        "sha256:0000000000000000000000000000000000000000000000000000000000000001".to_owned(),
    )])
}

fn valid_point() -> MetricPoint {
    MetricPoint {
        descriptor: canonical_descriptor("d2b_otel_ingress_policy_total").unwrap(),
        labels: BTreeMap::from([
            ("ingress".to_owned(), "emitter_unix".to_owned()),
            ("outcome".to_owned(), "accepted".to_owned()),
            ("error_class".to_owned(), "none".to_owned()),
        ]),
        value: 1.0,
    }
}

fn oversize_point() -> MetricPoint {
    let values = BTreeMap::from([
        ("verb".to_owned(), "get".to_owned()),
        ("resource_type".to_owned(), "Host".to_owned()),
        ("outcome".to_owned(), "ok".to_owned()),
    ]);
    MetricPoint {
        descriptor: canonical_descriptor("d2b_api_request_total").unwrap(),
        labels: values,
        value: 1.0,
    }
}

fn valid_frame() -> MetricFrame {
    MetricFrame::new(64, [valid_point()], valid_resource_attributes())
}

fn frame_with_label(key: &str, values: &[&str], value: &str) -> MetricFrame {
    MetricFrame::new(
        64,
        [MetricPoint {
            descriptor: MetricDescriptor::new(
                "d2b_otel_ingress_policy_total",
                [label(key, values)],
            ),
            labels: BTreeMap::from([(key.to_owned(), value.to_owned())]),
            value: 1.0,
        }],
        valid_resource_attributes(),
    )
}

fn expected_repeated_violation_outcome(ingress: Ingress, attempt: u8) -> IngressOutcome {
    match ingress {
        Ingress::EmitterUnix => IngressOutcome::Rejected,
        Ingress::OtlpUnix => {
            if attempt == QUARANTINE_VIOLATION_THRESHOLD {
                IngressOutcome::Quarantined
            } else {
                IngressOutcome::Rejected
            }
        }
        Ingress::OtlpVsock => {
            if attempt == QUARANTINE_VIOLATION_THRESHOLD {
                IngressOutcome::Quarantined
            } else {
                IngressOutcome::Rejected
            }
        }
        Ingress::ImportStream => {
            if attempt == QUARANTINE_VIOLATION_THRESHOLD {
                IngressOutcome::Quarantined
            } else {
                IngressOutcome::Rejected
            }
        }
    }
}

#[test]
fn every_ingress_covers_each_policy_failure_and_capacity_rejection() {
    for ingress in ALL_INGRESSES {
        for case in PolicyCase::ALL {
            let (frame, canaries, expected_error) = case.input();
            let mut gate = IngressPolicyGate::default();
            let actual = gate.admit_for_connection(ingress, 7, &frame, &canaries, true);

            assert_eq!(
                actual,
                (IngressOutcome::Rejected, expected_error),
                "{}/{}",
                ingress.as_str(),
                case.name()
            );
            assert!(
                !gate.is_connection_quarantined(ingress, 7),
                "{}/{} quarantined before the threshold",
                ingress.as_str(),
                case.name()
            );
        }

        let mut gate = IngressPolicyGate::default();
        assert_eq!(
            gate.admit_for_connection(
                ingress,
                7,
                &valid_frame(),
                &IdentityCanaries::default(),
                false
            ),
            (IngressOutcome::Rejected, IngressErrorClass::None),
            "{} rejected a valid frame for an unexpected reason",
            ingress.as_str()
        );
        assert!(
            !gate.is_connection_quarantined(ingress, 7),
            "{} quarantined a valid frame when capacity was unavailable",
            ingress.as_str()
        );
    }
}

#[test]
fn emitter_never_quarantines_and_streams_quarantine_at_the_threshold() {
    for ingress in ALL_INGRESSES {
        for case in PolicyCase::ALL {
            let (frame, canaries, expected_error) = case.input();
            let mut gate = IngressPolicyGate::default();

            for attempt in 1..=QUARANTINE_VIOLATION_THRESHOLD {
                let actual = gate.admit_for_connection(ingress, 7, &frame, &canaries, true);
                assert_eq!(
                    actual,
                    (
                        expected_repeated_violation_outcome(ingress, attempt),
                        expected_error,
                    ),
                    "{}/{} on policy violation {attempt}",
                    ingress.as_str(),
                    case.name()
                );
                assert_eq!(
                    gate.is_connection_quarantined(ingress, 7),
                    expected_repeated_violation_outcome(ingress, attempt)
                        == IngressOutcome::Quarantined,
                    "{}/{} quarantine state on policy violation {attempt}",
                    ingress.as_str(),
                    case.name()
                );
            }
        }
    }
}
