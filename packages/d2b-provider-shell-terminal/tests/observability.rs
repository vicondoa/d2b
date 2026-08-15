use d2b_provider_shell_terminal::{
    DiagnosticAccumulator, DiagnosticKind, ExecutionKind, ShellMetrics,
};

#[test]
fn observability_is_bounded_and_never_retains_terminal_payloads() {
    let mut diagnostics = DiagnosticAccumulator::new(2, 32).unwrap();
    diagnostics.record(DiagnosticKind::AttachDenied);
    diagnostics.record(DiagnosticKind::SupervisorLost);
    diagnostics.record(DiagnosticKind::AttachDenied);
    assert_eq!(diagnostics.len(), 2);
    assert!(!format!("{diagnostics:?}").contains("terminal"));

    let mut metrics = ShellMetrics::default();
    metrics.record_attach(ExecutionKind::Guest, false);
    assert_eq!(metrics.attach_denied(ExecutionKind::Guest), 1);
}
