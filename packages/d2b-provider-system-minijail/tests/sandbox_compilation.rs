use d2b_contracts_resource::v3::{
    CapabilityClass,
    EnvironmentClass,
    NamespaceClass,
    SandboxSpec,
    execution_policy::BoundedToken,
};
use d2b_process_conformance::ProcessConformanceError;
use d2b_provider_system_minijail::sandbox_compiler::MinijailSandboxCompiler;

fn sandbox(start_root: bool) -> SandboxSpec {
    SandboxSpec::new(
        vec![NamespaceClass::Mount, NamespaceClass::Pid],
        vec![CapabilityClass::Chown],
        BoundedToken::parse("strict").unwrap(),
        !start_root,
        start_root,
        EnvironmentClass::Minimal,
        true,
        Some("0022".to_owned()),
        0,
        None,
    )
    .unwrap()
}

#[test]
fn minijail_compiles_semantic_sandbox_without_raw_launch_details() {
    let plan = MinijailSandboxCompiler::default()
        .compile(
            &sandbox(false),
            d2b_contracts_resource::v3::execution_policy::ExecutionDomain::System,
        )
        .unwrap();
    assert!(plan.compiled.requires_cgroup_kill());
    assert_eq!(
        format!("{:?}", plan),
        "MinijailSandboxPlan { compiled: CompiledSandbox { digest: ConfigurationDigest(<redacted>), domain: System, requires_cgroup_kill: true }, user_namespace: None }"
    );
}

#[test]
fn start_root_requires_signed_provider_authorization() {
    let compiler = MinijailSandboxCompiler::default();
    assert_eq!(
        compiler
            .compile(
                &sandbox(true),
                d2b_contracts_resource::v3::execution_policy::ExecutionDomain::System
            )
            .unwrap_err(),
        ProcessConformanceError::SandboxRejected
    );
    assert!(
        compiler
            .compile_with_root_authorization(
                &sandbox(true),
                d2b_contracts_resource::v3::execution_policy::ExecutionDomain::System,
                true,
            )
            .is_ok()
    );
}
