//! Integration tests for d2b-wayland-filter.
//!
//! These tests exercise the policy engine, rewrite logic, and the wl-proxy
//! handler scaffolding without requiring a live Wayland compositor.

// Re-export policy types for the test module.
use d2b_wayland_filter::*;

mod policy_tests {
    use crate::*;

    #[test]
    fn policy_merge_required_plus_operator_deny() {
        let p = FilterPolicy::build(PolicyInput {
            vm_name: "test".to_owned(),
            deny_globals: vec!["wl_seat".to_owned()],
            ..Default::default()
        });
        // wl_seat is required — denying it must produce a warning.
        assert!(p.warnings.iter().any(|w| matches!(
            w,
            PolicyWarning::RequiredGlobalDenied { interface } if interface == "wl_seat"
        )));
        // The deny was actually applied.
        assert!(!p.is_allowed("wl_seat"));
    }

    #[test]
    fn policy_merge_operator_allow_overrides_default_deny() {
        let p = FilterPolicy::build(PolicyInput {
            vm_name: "test".to_owned(),
            allow_globals: vec!["zwlr_screencopy_manager_v1".to_owned()],
            ..Default::default()
        });
        assert!(p.is_allowed("zwlr_screencopy_manager_v1"));
    }

    #[test]
    fn policy_merge_version_cap_with_allow() {
        let p = FilterPolicy::build(PolicyInput {
            vm_name: "test".to_owned(),
            max_versions: vec![("zwp_linux_dmabuf_v1".to_owned(), 2)],
            ..Default::default()
        });
        assert_eq!(p.advertised_version("zwp_linux_dmabuf_v1", 5), 2);
        assert_eq!(p.advertised_version("zwp_linux_dmabuf_v1", 1), 1);
    }

    #[test]
    fn privileged_clipboard_primary_and_dnd_globals_are_denied() {
        let p = FilterPolicy::build(PolicyInput {
            vm_name: "test".to_owned(),
            ..Default::default()
        });

        for iface in [
            "ext_data_control_manager_v1",
            "zwlr_data_control_manager_v1",
            "zwp_primary_selection_device_manager_v1",
            "wp_primary_selection_device_manager_v1",
            "gtk_primary_selection_device_manager",
            "xdg_toplevel_drag_manager_v1",
        ] {
            assert!(!p.is_allowed(iface), "{iface} must be denied by default");
        }
    }
}

mod clipboard_boundary_tests {
    use d2b_wayland_filter::clipboard::{
        ClipboardGlobalDisposition, ClipboardMimePolicy, ClipboardObjectForwarding, ClipboardRoute,
        MimeDecision, global_disposition, object_forwarding,
    };

    #[test]
    fn standard_clipboard_objects_are_not_upstream_forwarded() {
        assert_eq!(
            global_disposition("wl_data_device_manager"),
            ClipboardGlobalDisposition::VirtualizeLocally
        );

        for iface in [
            "wl_data_device_manager",
            "wl_data_device",
            "wl_data_source",
            "wl_data_offer",
        ] {
            assert_eq!(
                object_forwarding(iface),
                ClipboardObjectForwarding::NeverForwardUpstream
            );
        }
    }

    #[test]
    fn same_vm_custom_mime_policy_placeholder_is_explicit() {
        let policy = ClipboardMimePolicy::v1_defaults();

        assert_eq!(
            policy.decide(ClipboardRoute::SameVm, "application/x-custom-rich"),
            MimeDecision::PreserveSameVmRichMime
        );
        assert_eq!(
            policy.decide(
                ClipboardRoute::HostOrCrossRealm,
                "application/x-custom-rich"
            ),
            MimeDecision::Deny
        );
    }
}
