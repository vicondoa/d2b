//! Retired gateway credential enrollment entrypoint.

fn main() -> std::process::ExitCode {
    d2b_gateway_runtime::provider_agent::run()
}
