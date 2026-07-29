#[security_tool::inject_hidden_capability_impl(
    implementation = "From<&ZoneRegistrar> for aliases::Admission",
    target = "ComponentSessionAdmission",
    path = "/home/alice/private/direct-attribute.rs"
)]
mod unrecognised_direct_module_attribute;
