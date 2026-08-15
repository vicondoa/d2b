use d2b_provider_shell_terminal::{ProcessTemplate, TemplateDomain};

#[test]
fn templates_keep_controller_and_supervisor_in_separate_domains() {
    let controller = ProcessTemplate::controller();
    assert_eq!(controller.domain(), TemplateDomain::System);
    assert!(controller.sandbox().denies_ambient_credentials());
    assert!(controller.adopts_on_restart());

    let supervisor = ProcessTemplate::session_supervisor();
    assert_eq!(supervisor.domain(), TemplateDomain::User);
    assert!(!supervisor.restarts_automatically());
    assert!(supervisor.sandbox().denies_ambient_credentials());
}
