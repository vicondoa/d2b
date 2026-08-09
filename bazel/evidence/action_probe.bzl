"""Small native action probes used to observe the pinned sandbox."""

def _d2b_action_probe_impl(ctx):
    output = ctx.actions.declare_file(ctx.label.name + ".out")
    ctx.actions.run_shell(
        outputs = [output],
        command = "printf '%s\\n' \"$1\" > \"$1\"",
        arguments = [output.path],
        mnemonic = ctx.attr.mnemonic,
        execution_requirements = {
            "no-cache": "1",
            "no-remote": "1",
            "no-network": "1",
            "d2b.execution_strategy": "sandboxed",
        },
    )
    return [DefaultInfo(files = depset([output]))]

d2b_action_probe = rule(
    implementation = _d2b_action_probe_impl,
    attrs = {
        "mnemonic": attr.string(mandatory = True),
    },
)

def _d2b_environment_probe_impl(ctx):
    output = ctx.actions.declare_file(ctx.label.name + ".txt")
    ctx.actions.run(
        executable = ctx.executable.tool,
        arguments = [output.path],
        outputs = [output],
        mnemonic = "EnvironmentProbe",
        execution_requirements = {
            "no-cache": "1",
            "no-remote": "1",
            "no-network": "1",
            "d2b.execution_strategy": "sandboxed",
        },
    )
    return [DefaultInfo(files = depset([output]))]

d2b_environment_probe = rule(
    implementation = _d2b_environment_probe_impl,
    attrs = {
        "tool": attr.label(
            default = Label("//bazel/evidence:environment-probe"),
            executable = True,
            cfg = "exec",
        ),
    },
)
