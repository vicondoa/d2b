//! Wayland global filter policy engine.
//!
//! The policy uses a four-layer classified allowlist. From lowest to highest
//! priority:
//!
//! 1. Required-baseline rules: core globals that nixling needs for graphics.
//! 2. Secure feature defaults: named bundles with safe on/off defaults.
//! 3. Custom operator features: operator-defined bundles.
//! 4. Explicit operator overrides: per-global allow/deny/version rules.
//!
//! Unknown globals (not in any layer) are denied by default.

use std::collections::HashMap;

/// The action to apply for a Wayland global interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalAction {
    Allow,
    Deny,
}

/// Classification used to decide whether a policy override triggers a warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// Core baseline global; warn if the operator denies it.
    RequiredBaseline,
    /// Global needed for accelerated rendering; warn if disabled.
    AcceleratedRendering,
    /// High-risk global; warn if the operator enables it.
    HighRisk,
    /// Ordinary app global that is on by default but not required.
    AppDefault,
    /// Global that is off by default but not considered high-risk.
    OffDefault,
    /// Global classified as unknown/unclassified; warn if explicitly allowed.
    Unclassified,
}

/// A resolved policy entry for a single interface.
#[derive(Debug, Clone)]
pub struct PolicyEntry {
    pub action: GlobalAction,
    /// Optional advertised-version cap. `None` means no cap beyond compositor's version.
    pub max_version: Option<u32>,
    pub classification: Classification,
}

/// A warning produced when the policy differs from secure defaults in a notable way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyWarning {
    RequiredGlobalDenied { interface: String },
    AcceleratedRenderingDisabled { interface: String },
    HighRiskGlobalEnabled { interface: String },
    AppIdPrefixNotVmPrefix { vm: String, prefix: String },
    TitlePrefixDisabled,
    UnclassifiedGlobalAllowed { interface: String },
}

impl PolicyWarning {
    /// Human-readable runtime advisory emitted by nixling-wayland-filter.
    pub fn message(&self) -> String {
        match self {
            Self::RequiredGlobalDenied { interface } => format!(
                "waylandFilter: required global `{interface}` is denied; graphics path may break"
            ),
            Self::AcceleratedRenderingDisabled { interface } => format!(
                "waylandFilter: accelerated-rendering global `{interface}` is denied; \
                 apps may fall back to software rendering"
            ),
            Self::HighRiskGlobalEnabled { interface } => format!(
                "waylandFilter: high-risk global `{interface}` is enabled; \
                 this global has elevated access to host compositor state"
            ),
            Self::AppIdPrefixNotVmPrefix { vm, prefix } => format!(
                "waylandFilter: appIdPrefix is `{prefix}` rather than the default \
                 `nixling.{vm}.`; generated niri border rules will not match unless \
                 overridden too"
            ),
            Self::TitlePrefixDisabled => {
                "waylandFilter: titlePrefix is empty; non-niri compositors lose VM \
                 disambiguation"
                    .to_owned()
            }
            Self::UnclassifiedGlobalAllowed { interface } => format!(
                "waylandFilter: unclassified global `{interface}` is explicitly allowed; \
                 nixling has not reviewed this protocol's security posture"
            ),
        }
    }
}

/// Per-global override instruction from the operator.
#[derive(Debug, Clone)]
pub struct GlobalOverride {
    pub interface: String,
    pub action: GlobalAction,
    pub max_version: Option<u32>,
}

/// Input configuration for policy construction.
#[derive(Debug, Clone, Default)]
pub struct PolicyInput {
    /// VM name, e.g. `work`.
    pub vm_name: String,
    /// Prefix prepended to `xdg_toplevel.set_app_id` values.
    /// Default: `nixling.<vm>.`
    pub app_id_prefix: Option<String>,
    /// Prefix prepended to `xdg_toplevel.set_title` values.
    /// Default: `[<vm>] `
    pub title_prefix: Option<String>,
    /// Additional explicit deny rules (appended after defaults).
    pub deny_globals: Vec<String>,
    /// Additional explicit allow rules (override default deny for a global).
    pub allow_globals: Vec<String>,
    /// Per-global version caps.
    pub max_versions: Vec<(String, u32)>,
    /// Emit a log line for every filtered global advertisement.
    pub log_filtered_globals: bool,
}

/// Fully resolved policy ready for use by filter handlers.
#[derive(Debug, Clone)]
pub struct FilterPolicy {
    entries: HashMap<String, PolicyEntry>,
    pub app_id_prefix: String,
    pub title_prefix: String,
    pub vm_name: String,
    pub log_filtered_globals: bool,
    /// Runtime advisories emitted by the filter process at startup.
    pub warnings: Vec<PolicyWarning>,
}

impl FilterPolicy {
    /// Build a policy from operator input layered on top of secure defaults.
    pub fn build(input: PolicyInput) -> Self {
        let vm = &input.vm_name;

        let app_id_prefix = input
            .app_id_prefix
            .unwrap_or_else(|| format!("nixling.{vm}."));
        let title_prefix = input.title_prefix.unwrap_or_else(|| format!("[{vm}] "));

        // Populate the default entries from the classified allowlist.
        let mut entries: HashMap<String, PolicyEntry> = default_classified_entries();

        // Apply operator explicit deny overrides.
        for iface in &input.deny_globals {
            if let Some(e) = entries.get_mut(iface.as_str()) {
                e.action = GlobalAction::Deny;
            } else {
                entries.insert(
                    iface.clone(),
                    PolicyEntry {
                        action: GlobalAction::Deny,
                        max_version: None,
                        classification: Classification::Unclassified,
                    },
                );
            }
        }

        // Apply operator explicit allow overrides.
        for iface in &input.allow_globals {
            if let Some(e) = entries.get_mut(iface.as_str()) {
                e.action = GlobalAction::Allow;
            } else {
                entries.insert(
                    iface.clone(),
                    PolicyEntry {
                        action: GlobalAction::Allow,
                        max_version: None,
                        classification: Classification::Unclassified,
                    },
                );
            }
        }

        // Apply version caps.
        for (iface, ver) in &input.max_versions {
            if let Some(e) = entries.get_mut(iface.as_str()) {
                e.max_version = Some(*ver);
            } else {
                entries.insert(
                    iface.clone(),
                    PolicyEntry {
                        action: GlobalAction::Allow,
                        max_version: Some(*ver),
                        classification: Classification::Unclassified,
                    },
                );
            }
        }

        // Collect warnings.
        let mut warnings: Vec<PolicyWarning> = Vec::new();

        // Check required baseline globals.
        for (iface, entry) in &entries {
            if entry.classification == Classification::RequiredBaseline
                && entry.action == GlobalAction::Deny
            {
                warnings.push(PolicyWarning::RequiredGlobalDenied {
                    interface: iface.clone(),
                });
            }
            if entry.classification == Classification::AcceleratedRendering
                && entry.action == GlobalAction::Deny
            {
                warnings.push(PolicyWarning::AcceleratedRenderingDisabled {
                    interface: iface.clone(),
                });
            }
            if entry.classification == Classification::HighRisk
                && entry.action == GlobalAction::Allow
            {
                warnings.push(PolicyWarning::HighRiskGlobalEnabled {
                    interface: iface.clone(),
                });
            }
            if entry.classification == Classification::Unclassified
                && entry.action == GlobalAction::Allow
            {
                warnings.push(PolicyWarning::UnclassifiedGlobalAllowed {
                    interface: iface.clone(),
                });
            }
        }

        let expected_app_id_prefix = format!("nixling.{vm}.");
        if app_id_prefix != expected_app_id_prefix && !app_id_prefix.is_empty() {
            warnings.push(PolicyWarning::AppIdPrefixNotVmPrefix {
                vm: vm.clone(),
                prefix: app_id_prefix.clone(),
            });
        }
        if title_prefix.is_empty() {
            warnings.push(PolicyWarning::TitlePrefixDisabled);
        }

        Self {
            entries,
            app_id_prefix,
            title_prefix,
            vm_name: vm.clone(),
            log_filtered_globals: input.log_filtered_globals,
            warnings,
        }
    }

    /// Look up the effective action and version cap for an interface.
    /// Returns `(GlobalAction::Deny, None)` for unclassified/unknown interfaces.
    pub fn lookup(&self, interface: &str) -> (GlobalAction, Option<u32>) {
        match self.entries.get(interface) {
            Some(e) => (e.action, e.max_version),
            None => (GlobalAction::Deny, None),
        }
    }

    /// Returns true if the policy allows this interface.
    pub fn is_allowed(&self, interface: &str) -> bool {
        self.lookup(interface).0 == GlobalAction::Allow
    }

    /// Returns the version to advertise: min of `compositor_version` and
    /// any policy `max_version` cap.
    pub fn advertised_version(&self, interface: &str, compositor_version: u32) -> u32 {
        match self.entries.get(interface).and_then(|e| e.max_version) {
            Some(cap) => compositor_version.min(cap),
            None => compositor_version,
        }
    }

    /// Rewrite an app-id value received from the guest.
    ///
    /// Rules:
    /// - If the value already starts with our exact VM prefix, pass through unchanged.
    /// - If the value starts with `nixling.<other>.`, prepend our prefix so it becomes
    ///   `nixling.<this>.nixling.<other>....` — spoof prevention.
    /// - Otherwise prepend our prefix unconditionally.
    pub fn rewrite_app_id(&self, guest_value: &str) -> String {
        if self.app_id_prefix.is_empty() {
            return guest_value.to_owned();
        }
        // Already has our exact prefix — pass through.
        if guest_value.starts_with(&self.app_id_prefix) {
            return guest_value.to_owned();
        }
        // Prepend our prefix (covers both plain values and cross-VM spoofs).
        format!("{}{}", self.app_id_prefix, guest_value)
    }

    /// Rewrite a window title received from the guest.
    ///
    /// Prepends `title_prefix` unless already present (idempotent).
    pub fn rewrite_title(&self, guest_value: &str) -> String {
        if self.title_prefix.is_empty() {
            return guest_value.to_owned();
        }
        if guest_value.starts_with(&self.title_prefix) {
            return guest_value.to_owned();
        }
        format!("{}{}", self.title_prefix, guest_value)
    }
}

/// Populate the default classified entry table.
///
/// The map is keyed by interface name string so it matches the names returned
/// by `ObjectInterface::name()` at runtime.
fn default_classified_entries() -> HashMap<String, PolicyEntry> {
    let mut m = HashMap::new();

    macro_rules! entry {
        ($iface:expr, $action:ident, $class:ident) => {
            m.insert(
                $iface.to_owned(),
                PolicyEntry {
                    action: GlobalAction::$action,
                    max_version: None,
                    classification: Classification::$class,
                },
            );
        };
        ($iface:expr, $action:ident, $class:ident, max=$v:expr) => {
            m.insert(
                $iface.to_owned(),
                PolicyEntry {
                    action: GlobalAction::$action,
                    max_version: Some($v),
                    classification: Classification::$class,
                },
            );
        };
    }

    // --- baseline-app (required, enabled) ---
    entry!("wl_compositor", Allow, RequiredBaseline);
    entry!("wl_shm", Allow, RequiredBaseline);
    entry!("wl_seat", Allow, RequiredBaseline);
    entry!("xdg_wm_base", Allow, RequiredBaseline);
    entry!("wl_output", Allow, RequiredBaseline);
    entry!("wl_subcompositor", Allow, RequiredBaseline);
    entry!("wl_data_device_manager", Allow, AppDefault);

    // --- accelerated-rendering (enabled, warn if denied) ---
    entry!("zwp_linux_dmabuf_v1", Allow, AcceleratedRendering);
    entry!(
        "wp_linux_drm_syncobj_manager_v1",
        Allow,
        AcceleratedRendering
    );
    entry!("wl_eglstream_display", Allow, AcceleratedRendering);
    entry!("wl_eglstream_controller", Allow, AcceleratedRendering);
    entry!("wp_single_pixel_buffer_v1", Allow, AppDefault);

    // --- presentation-and-scaling (enabled, app default) ---
    entry!("wp_presentation", Allow, AppDefault);
    entry!("wp_fractional_scale_manager_v1", Allow, AppDefault);
    entry!("wp_viewporter", Allow, AppDefault);
    entry!("zxdg_decoration_manager_v1", Allow, AppDefault);
    entry!("xdg_activation_v1", Allow, AppDefault);
    entry!("wp_content_type_manager_v1", Allow, AppDefault);
    entry!("wp_cursor_shape_manager_v1", Allow, AppDefault);
    entry!("wp_commit_timing_manager_v1", Allow, AppDefault);
    entry!("wp_fifo_manager_v1", Allow, AppDefault);
    entry!("wp_alpha_modifier_v1", Allow, AppDefault);
    entry!("wp_tearing_control_manager_v1", Allow, AppDefault);
    entry!("xdg_output_unstable_v1", Allow, AppDefault);
    entry!("xdg_system_bell_v1", Allow, AppDefault);
    entry!("zxdg_output_manager_v1", Allow, AppDefault);
    entry!("ext_idle_notifier_v1", Allow, AppDefault);
    entry!("zwp_idle_inhibit_manager_v1", Allow, AppDefault);
    entry!("wp_color_manager_v1", Allow, AppDefault);
    entry!("xdg_dialog_v1", Allow, AppDefault);
    entry!("xdg_wm_dialog_v1", Allow, AppDefault);
    entry!("xdg_toplevel_icon_manager_v1", Allow, AppDefault);
    entry!("xdg_toplevel_drag_manager_v1", Allow, AppDefault);
    entry!("xdg_activation_token_v1", Allow, AppDefault);
    entry!("xdg_toplevel_tag_manager_v1", Allow, AppDefault);

    // Input protocols — standard app-level input
    entry!("zwp_relative_pointer_manager_v1", Allow, AppDefault);
    entry!("zwp_pointer_constraints_v1", Allow, AppDefault);
    entry!("zwp_pointer_gestures_v1", Allow, AppDefault);
    entry!("zwp_tablet_manager_v2", Allow, AppDefault);
    entry!("zwp_text_input_manager_v3", Allow, AppDefault);
    entry!("zwp_input_timestamps_manager_v1", Allow, AppDefault);
    entry!(
        "zwp_keyboard_shortcuts_inhibit_manager_v1",
        Allow,
        AppDefault
    );
    entry!("wp_pointer_warp_v1", Allow, AppDefault);

    // wl_drm is legacy but still used by some Mesa paths
    entry!("wl_drm", Allow, AppDefault);

    // --- screen-capture (disabled by default, high-risk) ---
    entry!("zwlr_screencopy_manager_v1", Deny, HighRisk);
    entry!("ext_image_copy_capture_manager_v1", Deny, HighRisk);
    entry!("ext_image_capture_source_v1", Deny, HighRisk);
    entry!("ext_output_image_capture_source_manager_v1", Deny, HighRisk);
    entry!(
        "ext_foreign_toplevel_image_capture_source_manager_v1",
        Deny,
        HighRisk
    );

    // --- virtual-input (disabled by default, high-risk) ---
    entry!("zwp_virtual_keyboard_manager_v1", Deny, HighRisk);
    entry!("zwlr_virtual_pointer_manager_v1", Deny, HighRisk);

    // --- clipboard-control (disabled by default, high-risk) ---
    entry!("ext_data_control_manager_v1", Deny, HighRisk);
    entry!("zwlr_data_control_manager_v1", Deny, HighRisk);
    entry!("zwp_primary_selection_device_manager_v1", Deny, HighRisk);
    entry!("wp_primary_selection_unstable_v1", Deny, HighRisk);

    // --- desktop-shell (disabled by default, high-risk) ---
    entry!("zwlr_layer_shell_v1", Deny, HighRisk);

    // --- session-control (disabled by default, high-risk) ---
    entry!("ext_session_lock_manager_v1", Deny, HighRisk);
    entry!("zwlr_input_inhibit_manager_v1", Deny, HighRisk);
    entry!("zwlr_output_manager_v1", Deny, HighRisk);
    entry!("zwlr_output_power_manager_v1", Deny, HighRisk);
    entry!("zwlr_gamma_control_manager_v1", Deny, HighRisk);
    entry!("ext_workspace_manager_v1", Deny, HighRisk);
    entry!("zwlr_foreign_toplevel_manager_v1", Deny, HighRisk);
    entry!("ext_foreign_toplevel_list_v1", Deny, HighRisk);

    // --- security-context (disabled by default) ---
    entry!("wp_security_context_manager_v1", Deny, OffDefault);

    // Legacy wl_shell — disabled
    entry!("wl_shell", Deny, OffDefault);

    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy_for(vm: &str) -> FilterPolicy {
        FilterPolicy::build(PolicyInput {
            vm_name: vm.to_owned(),
            ..Default::default()
        })
    }

    #[test]
    fn secure_defaults_produce_no_warnings() {
        let p = policy_for("work");
        assert!(
            p.warnings.is_empty(),
            "secure defaults must produce zero warnings; got: {:?}",
            p.warnings
        );
    }

    #[test]
    fn required_globals_are_allowed() {
        let p = policy_for("work");
        for iface in &[
            "wl_compositor",
            "wl_shm",
            "wl_seat",
            "xdg_wm_base",
            "wl_output",
        ] {
            assert!(
                p.is_allowed(iface),
                "required global `{iface}` must be allowed by default"
            );
        }
    }

    #[test]
    fn high_risk_globals_are_denied() {
        let p = policy_for("work");
        for iface in &[
            "zwlr_screencopy_manager_v1",
            "zwp_virtual_keyboard_manager_v1",
            "ext_data_control_manager_v1",
            "zwlr_layer_shell_v1",
            "ext_session_lock_manager_v1",
        ] {
            assert!(
                !p.is_allowed(iface),
                "high-risk global `{iface}` must be denied by default"
            );
        }
    }

    #[test]
    fn unknown_globals_are_denied() {
        let p = policy_for("work");
        assert!(!p.is_allowed("completely_unknown_protocol_v1"));
        assert!(!p.is_allowed("zwp_some_future_compositor_control_v1"));
    }

    #[test]
    fn deny_required_global_produces_warning() {
        let p = FilterPolicy::build(PolicyInput {
            vm_name: "work".to_owned(),
            deny_globals: vec!["wl_compositor".to_owned()],
            ..Default::default()
        });
        assert!(p.warnings.iter().any(|w| matches!(
            w,
            PolicyWarning::RequiredGlobalDenied { interface }
            if interface == "wl_compositor"
        )));
    }

    #[test]
    fn deny_accelerated_rendering_produces_warning() {
        let p = FilterPolicy::build(PolicyInput {
            vm_name: "work".to_owned(),
            deny_globals: vec!["zwp_linux_dmabuf_v1".to_owned()],
            ..Default::default()
        });
        assert!(p.warnings.iter().any(|w| matches!(
            w,
            PolicyWarning::AcceleratedRenderingDisabled { interface }
            if interface == "zwp_linux_dmabuf_v1"
        )));
    }

    #[test]
    fn nvidia_eglstream_global_is_accelerated_rendering() {
        let p = policy_for("work");
        assert!(p.is_allowed("wl_eglstream_display"));
        assert!(p.is_allowed("wl_eglstream_controller"));

        let p = FilterPolicy::build(PolicyInput {
            vm_name: "work".to_owned(),
            deny_globals: vec![
                "wl_eglstream_display".to_owned(),
                "wl_eglstream_controller".to_owned(),
            ],
            ..Default::default()
        });
        assert!(p.warnings.iter().any(|w| matches!(
            w,
            PolicyWarning::AcceleratedRenderingDisabled { interface }
            if interface == "wl_eglstream_display"
        )));
        assert!(p.warnings.iter().any(|w| matches!(
            w,
            PolicyWarning::AcceleratedRenderingDisabled { interface }
            if interface == "wl_eglstream_controller"
        )));
    }

    #[test]
    fn enable_high_risk_global_produces_warning() {
        let p = FilterPolicy::build(PolicyInput {
            vm_name: "work".to_owned(),
            allow_globals: vec!["zwlr_screencopy_manager_v1".to_owned()],
            ..Default::default()
        });
        assert!(p.warnings.iter().any(|w| matches!(
            w,
            PolicyWarning::HighRiskGlobalEnabled { interface }
            if interface == "zwlr_screencopy_manager_v1"
        )));
    }

    #[test]
    fn allow_unclassified_global_produces_warning() {
        let p = FilterPolicy::build(PolicyInput {
            vm_name: "work".to_owned(),
            allow_globals: vec!["completely_unknown_v1".to_owned()],
            ..Default::default()
        });
        assert!(p.warnings.iter().any(|w| matches!(
            w,
            PolicyWarning::UnclassifiedGlobalAllowed { interface }
            if interface == "completely_unknown_v1"
        )));
    }

    #[test]
    fn warnings_are_advisory_not_panics() {
        // Building a policy with multiple warning conditions must succeed.
        let p = FilterPolicy::build(PolicyInput {
            vm_name: "work".to_owned(),
            deny_globals: vec!["wl_compositor".to_owned(), "zwp_linux_dmabuf_v1".to_owned()],
            allow_globals: vec![
                "zwlr_screencopy_manager_v1".to_owned(),
                "totally_unknown_v1".to_owned(),
            ],
            app_id_prefix: Some(String::new()),
            title_prefix: Some(String::new()),
            ..Default::default()
        });
        // Some warnings are expected but the policy must be built successfully.
        assert!(!p.warnings.is_empty());
    }

    // --- app-id prefix tests ---

    #[test]
    fn app_id_plain_value_gets_prefix() {
        let p = policy_for("work");
        assert_eq!(
            p.rewrite_app_id("org.mozilla.firefox"),
            "nixling.work.org.mozilla.firefox"
        );
    }

    #[test]
    fn app_id_already_prefixed_passes_through() {
        let p = policy_for("work");
        let already = "nixling.work.org.mozilla.firefox";
        assert_eq!(p.rewrite_app_id(already), already);
    }

    #[test]
    fn app_id_cross_vm_spoof_gets_double_prefix() {
        let p = policy_for("work");
        // Guest sends a value pre-prefixed for a different VM — must not be
        // accepted as already-prefixed for this VM.
        let spoof = "nixling.personal.org.example.app";
        assert_eq!(
            p.rewrite_app_id(spoof),
            "nixling.work.nixling.personal.org.example.app"
        );
    }

    #[test]
    fn app_id_prefix_empty_passthrough() {
        let p = FilterPolicy::build(PolicyInput {
            vm_name: "work".to_owned(),
            app_id_prefix: Some(String::new()),
            title_prefix: Some("[work] ".to_owned()),
            ..Default::default()
        });
        assert_eq!(p.rewrite_app_id("org.example.app"), "org.example.app");
    }

    // --- title prefix tests ---

    #[test]
    fn title_plain_value_gets_prefix() {
        let p = policy_for("work");
        assert_eq!(p.rewrite_title("Firefox"), "[work] Firefox");
    }

    #[test]
    fn title_already_prefixed_passes_through() {
        let p = policy_for("work");
        let already = "[work] Firefox";
        assert_eq!(p.rewrite_title(already), already);
    }

    #[test]
    fn title_prefix_empty_passthrough() {
        let p = FilterPolicy::build(PolicyInput {
            vm_name: "work".to_owned(),
            title_prefix: Some(String::new()),
            ..Default::default()
        });
        assert_eq!(p.rewrite_title("Firefox"), "Firefox");
    }

    // --- version cap tests ---

    #[test]
    fn version_cap_applied() {
        let p = FilterPolicy::build(PolicyInput {
            vm_name: "work".to_owned(),
            max_versions: vec![("xdg_wm_base".to_owned(), 3)],
            ..Default::default()
        });
        assert_eq!(p.advertised_version("xdg_wm_base", 6), 3);
        assert_eq!(p.advertised_version("xdg_wm_base", 2), 2);
    }

    #[test]
    fn no_cap_passes_compositor_version() {
        let p = policy_for("work");
        assert_eq!(p.advertised_version("wl_compositor", 5), 5);
    }

    // --- lookup tests ---

    #[test]
    fn lookup_returns_deny_for_unknown() {
        let p = policy_for("work");
        assert_eq!(p.lookup("zwp_never_heard_of_this_v1").0, GlobalAction::Deny);
    }
}
