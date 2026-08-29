//! Cloud Hypervisor Provider controller ResourceV3 entry point.

fn main() {
    std::process::exit(
        d2b_provider_runtime_cloud_hypervisor::controller_binary_entrypoint(),
    );
}
