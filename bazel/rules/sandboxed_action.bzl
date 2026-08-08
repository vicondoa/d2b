"""Closed action policy for governed Rust actions.

The actual seccomp filter is carried by the pinned Nix Bazel package.  This
file keeps the Starlark side honest: governed actions have one strategy,
one network setting, and no ordered fallback list.
"""

D2B_ACTION_NETWORK = "none"
D2B_SANDBOX_STRATEGY = "sandboxed"
D2B_FORBIDDEN_STRATEGIES = [
    "process",
    "processwrapper",
    "local",
    "standalone",
    "worker",
    "remote",
    "no-sandbox",
]
D2B_STRATEGY_OVERRIDE_KEYS = [
    "strategy",
    "spawn_strategy",
    "test_strategy",
    "genrule_strategy",
    "worker_strategy",
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

def d2b_validate_effective_strategies(observations):
    """Validate the strategy selected by a real configured/aquery observation."""

    if not observations:
        fail("governed Bazel actions require non-empty strategy observations")
    for action, strategy in observations.items():
        d2b_validate_strategy(strategy)
        if strategy in D2B_FORBIDDEN_STRATEGIES:
            fail("governed Bazel actions do not permit strategy fallbacks")

def d2b_sandboxed_kwargs(kwargs = {}):
    """Add the invariant action attributes without accepting a fallback."""

    result = dict(kwargs)
    for key in D2B_STRATEGY_OVERRIDE_KEYS:
        if key in result:
            fail("governed Bazel actions do not accept strategy overrides")
    result["tags"] = sorted(set(result.get("tags", []) + d2b_sandbox_tags()))
    properties = dict(result.get("exec_properties", {}))
    if properties.get("network") not in [None, D2B_ACTION_NETWORK]:
        fail("governed Bazel actions cannot enable network access")
    if properties.get("d2b.execution_strategy") not in [None, D2B_SANDBOX_STRATEGY]:
        fail("governed Bazel actions cannot override the patched strategy")
    properties["network"] = D2B_ACTION_NETWORK
    properties["d2b.execution_strategy"] = D2B_SANDBOX_STRATEGY
    result["exec_properties"] = properties
    return result

def d2b_sandboxed_action(rule, name, **kwargs):
    """Apply the closed policy to one rule invocation."""

    rule(name = name, **d2b_sandboxed_kwargs(kwargs))

sandboxed_action = d2b_sandboxed_action
