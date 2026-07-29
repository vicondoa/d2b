#[cfg_attr(
    any(),
    cfg_attr(
        all(),
        security_tool::inject_hidden_capability_impl(
            implementation = "From<&ZoneRegistrar> for second::Admission",
            target = "ComponentSessionAdmission",
            path = "/home/alice/private/attribute.rs"
        )
    )
)]
mod unrecognised_module_cfg_attr;
