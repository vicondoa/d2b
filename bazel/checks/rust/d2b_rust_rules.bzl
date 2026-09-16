"""d2b_rust_rules: per-crate clippy gating for every Bazel Rust target.

Wraps rules_rust so every crate's Bazel rust target auto-emits a
`rust_clippy_test` folded into the crate's gate suite, giving Bazel a clippy
gate over every workspace crate (previously only `cargo clippy` covered it, so
a clippy regression never failed the Layer-1 gate or CI `rust-*` jobs).

Wrappers are transparent passthrough: every attribute a `rust_*` rule accepts
is forwarded unchanged; the macro only emits the per-target `_clippy` test.

Lint enforcement: the workspace `.bazelrc` sets
`per_crate_rustc_flag=//@-Dwarnings`, which the rules_rust clippy action picks
up through `construct_arguments` -> `collect_extra_rustc_flags`; with no
`lint_config` the clippy action also falls back to `-Dwarnings`. Bazel clippy
therefore hard-fails on any clippy warning exactly where the workspace's
`-Dwarnings` rustc policy already fails rustc warnings. The corpus is clean
(0 errors, 0 warnings under the manifest lint table), so the gate is a
regression tripwire, not a backlog cleanup.

NOTE (deviation from the original plan): the plan proposed attaching
`extract_cargo_lints` as each target's `lint_config` so Bazel clippy uses the
crate's exact manifest lint table. That mechanism is not loadable here: this
rules_rust copy ships no `cargo_toml_info` binary (the rule's private tool) and
d2b's `@crates` has no toml parser to rebuild it. Per decision, `lint_config`
is dropped and Bazel clippy relies on `-Dwarnings` over the empty rules_rust
table; the workspace clippy.toml (which arms `disallowed_methods`) is left
unwired so the allow-level backlog stays suppressed on both sides.
"""
load(
    "@rules_rust//rust:defs.bzl",
    _rust_binary = "rust_binary",
    _rust_clippy_test = "rust_clippy_test",
    _rust_doc_test = "rust_doc_test",
    _rust_library = "rust_library",
    _rust_test = "rust_test",
)

def _emit_clippy(name, clippy):
    # `transitive = False` lints exactly this crate's own sources, matching
    # cargo clippy per-crate. With no `lint_config` the clippy action runs
    # `-Dwarnings`, so any clippy warning fails the gate.
    if clippy:
        _rust_clippy_test(
            name = name + "_clippy",
            targets = [":" + name],
            transitive = False,
        )

def d2b_rust_library(name, srcs = [], compile_data = [], clippy = True, **kwargs):
    _rust_library(name = name, srcs = srcs, compile_data = compile_data, **kwargs)
    _emit_clippy(name, clippy)

def d2b_rust_binary(name, srcs = [], compile_data = [], clippy = True, **kwargs):
    _rust_binary(name = name, srcs = srcs, compile_data = compile_data, **kwargs)
    _emit_clippy(name, clippy)

def d2b_rust_test(name, srcs = [], compile_data = [], clippy = True, **kwargs):
    _rust_test(name = name, srcs = srcs, compile_data = compile_data, **kwargs)
    _emit_clippy(name, clippy)

def d2b_rust_doc_test(**kwargs):
    # Doc tests are cross-crate and not clippy'd (matches cargo).
    _rust_doc_test(**kwargs)