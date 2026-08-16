load("@rules_rust//rust:defs.bzl", "rust_doc_test")

def broker_features():
    return select({
        "//bazel/checks/rust:broker-layer1": ["layer1-bootstrap"],
        "//bazel/checks/rust:broker-fake-backends": ["fake-backends"],
        "//conditions:default": [],
    })


def broker_test_tags():
    return ["exclusive", "broker-process-global"]


def guest_features():
    return select({
        "//bazel/checks/rust:guest-real-libshpool": ["real-libshpool"],
        "//conditions:default": [],
    })


def rust_doc_carrier(name, crate, crate_features = []):
    rust_doc_test(
        name = name,
        crate = crate,
        crate_features = crate_features,
        visibility = ["//visibility:public"],
    )
