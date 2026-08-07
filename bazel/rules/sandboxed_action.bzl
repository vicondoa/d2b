"""Closed action policy for governed Rust actions.

The actual seccomp filter is carried by the pinned Nix Bazel package.  This
file keeps the Starlark side honest: governed actions have one strategy,
one network setting, and no ordered fallback list.
"""

D2B_ACTION_NETWORK = "none"
D2B_SANDBOX_STRATEGY = "sandboxed"
D2B_FORBIDDEN_STRATEGIES = [
    "process",
    "local",
    "standalone",
    "worker",
    "remote",
    "no-sandbox",
]

def d2b_sandbox_tags():
    """Return the closed tags attached to a governed action."""

    return ["d2b-sandboxed", "no-network"]

def d2b_validate_strategy(strategy):
    """Fail closed if a caller attempts a non-patched execution strategy."""

    if strategy != D2B_SANDBOX_STRATEGY:
        fail("governed Bazel actions require the patched Linux sandbox")
    if strategy in D2B_FORBIDDEN_STRATEGIES:
        fail("governed Bazel actions do not permit strategy fallbacks")

def d2b_sandboxed_kwargs(kwargs = {}):
    """Add the invariant action attributes without accepting a fallback."""

    d2b_validate_strategy(D2B_SANDBOX_STRATEGY)
    result = dict(kwargs)
    result["tags"] = sorted(set(result.get("tags", []) + d2b_sandbox_tags()))
    if result.get("exec_properties", {}).get("network") not in [None, D2B_ACTION_NETWORK]:
        fail("governed Bazel actions cannot enable network access")
    properties = dict(result.get("exec_properties", {}))
    properties["network"] = D2B_ACTION_NETWORK
    result["exec_properties"] = properties
    return result

def d2b_sandboxed_action(rule, name, **kwargs):
    """Apply the closed policy to one rule invocation."""

    rule(name = name, **d2b_sandboxed_kwargs(kwargs))

sandboxed_action = d2b_sandboxed_action
