//! Workload target index: maps realm-native identifiers to legacy VM names.
//!
//! The index is built once per public request from the realm controllers
//! config and provides three lookup operations:
//!
//! - canonical target (`workload.realm.d2b`) → legacy VM name
//! - legacy VM name → [`WorkloadIdentity`] (for list/status output)
//!
//! All lookups are fail-closed: canonical targets that do not exist return
//! [`TargetResolutionError::NotFound`]. Bare workload identifiers are never
//! interpreted as aliases.
//!
//! Only workloads with an `identity` field populated in the realm controllers
//! config contribute to the index. Workloads with `identity: None` (pre-W15
//! Nix emitters) are silently skipped, preserving full backwards compatibility.

use std::collections::HashMap;

use d2b_core::{
    realm_controller_config::RealmControllersJson, workload_identity::WorkloadIdentity,
};

/// Result of resolving an incoming target string to a legacy VM name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetResolution {
    /// String was already a known legacy VM name; no translation needed.
    LegacyVmName(String),
    /// Canonical workload target (`workload.realm.d2b`) resolved to a legacy VM name.
    ResolvedFromCanonicalTarget {
        canonical_target: String,
        vm_name: String,
    },
}

impl TargetResolution {
    /// Return the resolved legacy VM name regardless of resolution path.
    pub fn vm_name(&self) -> &str {
        match self {
            Self::LegacyVmName(name) => name,
            Self::ResolvedFromCanonicalTarget { vm_name, .. } => vm_name,
        }
    }
}

/// Error returned when workload target resolution fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetResolutionError {
    /// A canonical target (`something.d2b`) was supplied but is not in the index.
    NotFound { target: String },
    /// A workload id matched more than one workload — fail closed on ambiguity.
    AliasConflict {
        workload_id: String,
        candidates: Vec<String>,
    },
}

impl TargetResolutionError {
    /// Short human-readable message suitable for error log/response payloads.
    pub fn message(&self) -> String {
        match self {
            Self::NotFound { target } => {
                format!("workload target '{target}' not found in realm workload index")
            }
            Self::AliasConflict {
                workload_id,
                candidates,
            } => {
                format!(
                    "workload id '{workload_id}' is ambiguous: matches [{}]; \
                     use the canonical target (e.g. {workload_id}.realm.d2b) to disambiguate",
                    candidates.join(", ")
                )
            }
        }
    }
}

/// Lightweight index built from realm controller workload metadata.
///
/// Build with [`WorkloadTargetIndex::build_from_controllers`] once per public
/// request. The index is intentionally cheap to construct — it does only two
/// HashMap inserts per workload entry.
#[derive(Debug, Default, Clone)]
pub struct WorkloadTargetIndex {
    /// canonical target string → legacy VM name
    by_canonical_target: HashMap<String, String>,
    /// legacy VM name → WorkloadIdentity (for list/status injection)
    by_vm_name: HashMap<String, WorkloadIdentity>,
}

impl WorkloadTargetIndex {
    /// Build a `WorkloadTargetIndex` from a loaded realm controllers config.
    ///
    /// Only workloads with a populated `identity` field are indexed; others are
    /// silently skipped so the index is always a strict forward-compatible subset.
    pub fn build_from_controllers(config: &RealmControllersJson) -> Self {
        let mut index = Self::default();
        for controller in &config.controllers {
            let Some(local_runtime) = &controller.local_runtime else {
                continue;
            };
            for workload in &local_runtime.workloads {
                let Some(identity) = &workload.identity else {
                    continue;
                };
                let vm_name = workload.vm_name.as_str().to_owned();
                let canonical = identity.canonical_target.to_canonical();

                index.by_canonical_target.insert(canonical, vm_name.clone());
                index.by_vm_name.insert(vm_name, identity.clone());
            }
        }
        index
    }

    /// Return `true` if the index contains no entries.
    pub fn is_empty(&self) -> bool {
        self.by_vm_name.is_empty()
    }

    /// Look up the [`WorkloadIdentity`] for a legacy VM name.
    ///
    /// Returns `None` for VMs that have no associated realm workload identity
    /// (legacy-only entries not yet adopted into a realm).
    pub fn identity_for_vm(&self, vm_name: &str) -> Option<&WorkloadIdentity> {
        self.by_vm_name.get(vm_name)
    }

    /// Resolve an incoming target string to a legacy VM name.
    ///
    /// Resolution order:
    /// 1. If the string ends with `.d2b`, treat it as a canonical workload
    ///    target and look it up by exact match — returns `NotFound` if absent.
    /// 2. If the string is a known legacy VM name (present in
    ///    `known_legacy_vm_names`), pass it through unchanged.
    /// 3. Fall through as a `LegacyVmName` (caller is responsible for
    ///    validating that it actually exists in the manifest).
    ///
    /// The `known_legacy_vm_names` set is the caller's manifest key set; it
    /// distinguishes explicitly known legacy names from canonical targets.
    pub fn resolve_target(
        &self,
        target: &str,
        known_legacy_vm_names: &std::collections::HashSet<String>,
    ) -> Result<TargetResolution, TargetResolutionError> {
        // Step 1: canonical target (ends with .d2b).
        if target.ends_with(".d2b") {
            return match self.by_canonical_target.get(target) {
                Some(vm_name) => Ok(TargetResolution::ResolvedFromCanonicalTarget {
                    canonical_target: target.to_owned(),
                    vm_name: vm_name.clone(),
                }),
                None => Err(TargetResolutionError::NotFound {
                    target: target.to_owned(),
                }),
            };
        }

        // Step 2: already a known legacy VM name — fast path.
        if known_legacy_vm_names.contains(target) {
            return Ok(TargetResolution::LegacyVmName(target.to_owned()));
        }

        // Bare identifiers are legacy VM names only; there is no realm alias.
        Ok(TargetResolution::LegacyVmName(target.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    use d2b_core::realm_controller_config::RealmControllersJson;

    /// Minimal realm-controllers JSON with no local_runtime workloads.
    fn controllers_json_no_workloads() -> RealmControllersJson {
        serde_json::from_str(CONTROLLERS_TEMPLATE_NO_WORKLOADS).expect("parse controllers json")
    }

    /// Build a realm-controllers JSON with one or two workloads injected into
    /// the `localRuntime.workloads` array.
    fn controllers_json_with_workloads(workloads_json: &str) -> RealmControllersJson {
        let raw = format!(
            r#"{{
              "schemaVersion": "v2",
              "runtimeState": "metadata-only",
              "controllers": [
                {{
                  "realmName": "Work",
                  "realmId": "work",
                  "realmPath": "work",
                  "placement": "host-local",
                  "daemon": {{
                    "user": "d2br-work",
                    "group": "d2br-work",
                    "publicSocketGroup": "d2br-work",
                    "serviceName": "d2b-realm-work-daemon.service",
                    "configPath": "/etc/d2b/realms/work/daemon-config.json",
                    "stateLockPath": "/run/d2b/realms/work/daemon.lock",
                    "locksDir": "/run/d2b/realms/work/locks",
                    "socketActivated": false,
                    "materializedService": false
                  }},
                  "broker": {{
                    "enabled": false,
                    "hostMutation": false,
                    "user": "root",
                    "group": "d2br-work",
                    "socketPath": "/run/d2b/realms/work/priv.sock",
                    "socketUnitName": "d2b-realm-work-priv-broker.socket",
                    "serviceUnitName": "d2b-realm-work-priv-broker.service",
                    "auditDir": "/var/lib/d2b/realms/work/audit",
                    "materializedSocket": false,
                    "materializedService": false
                  }},
                  "paths": {{
                    "runDir": "/run/d2b/realms/work",
                    "stateDir": "/var/lib/d2b/realms/work",
                    "auditDir": "/var/lib/d2b/realms/work/audit"
                  }},
                  "sockets": {{
                    "publicSocketPath": "/run/d2b/realms/work/public.sock",
                    "brokerSocketPath": "/run/d2b/realms/work/priv.sock"
                  }},
                  "allocator": {{
                    "kind": "local-root-metadata",
                    "configPath": "/etc/d2b/allocator.json",
                    "rootSocket": "/run/d2b/allocator.sock"
                  }},
                  "access": {{}},
                  "localRuntime": {{
                    "runtimeState": "metadata-only",
                    "invariants": {{
                      "metadataOnly": true,
                      "existingGlobalVmPathsPreserved": true,
                      "noStateMigrationDuringActivation": true,
                      "brokerEffectsRemainRealmDelegated": true
                    }},
                    "workloads": {workloads_json}
                  }}
                }}
              ],
              "invariants": {{
                "metadataOnly": true,
                "noSystemdUnitsMaterialized": true,
                "preservesGlobalDaemonBehavior": true,
                "preservesDirectUnixSocketSemantics": true
              }}
            }}"#
        );
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse controllers json: {e}\n{raw}"))
    }

    const CONTROLLERS_TEMPLATE_NO_WORKLOADS: &str = r#"{
      "schemaVersion": "v2",
      "runtimeState": "metadata-only",
      "controllers": [
        {
          "realmName": "Work",
          "realmId": "work",
          "realmPath": "work",
          "placement": "host-local",
          "daemon": {
            "user": "d2br-work",
            "group": "d2br-work",
            "publicSocketGroup": "d2br-work",
            "serviceName": "d2b-realm-work-daemon.service",
            "configPath": "/etc/d2b/realms/work/daemon-config.json",
            "stateLockPath": "/run/d2b/realms/work/daemon.lock",
            "locksDir": "/run/d2b/realms/work/locks",
            "socketActivated": false,
            "materializedService": false
          },
          "broker": {
            "enabled": false,
            "hostMutation": false,
            "user": "root",
            "group": "d2br-work",
            "socketPath": "/run/d2b/realms/work/priv.sock",
            "socketUnitName": "d2b-realm-work-priv-broker.socket",
            "serviceUnitName": "d2b-realm-work-priv-broker.service",
            "auditDir": "/var/lib/d2b/realms/work/audit",
            "materializedSocket": false,
            "materializedService": false
          },
          "paths": {
            "runDir": "/run/d2b/realms/work",
            "stateDir": "/var/lib/d2b/realms/work",
            "auditDir": "/var/lib/d2b/realms/work/audit"
          },
          "sockets": {
            "publicSocketPath": "/run/d2b/realms/work/public.sock",
            "brokerSocketPath": "/run/d2b/realms/work/priv.sock"
          },
          "allocator": {
            "kind": "local-root-metadata",
            "configPath": "/etc/d2b/allocator.json",
            "rootSocket": "/run/d2b/allocator.sock"
          },
          "access": {}
        }
      ],
      "invariants": {
        "metadataOnly": true,
        "noSystemdUnitsMaterialized": true,
        "preservesGlobalDaemonBehavior": true,
        "preservesDirectUnixSocketSemantics": true
      }
    }"#;

    /// JSON for a workload with a full WorkloadIdentity.
    fn workload_with_identity(workload_id: &str, realm: &str, vm_name: &str) -> String {
        format!(
            r#"{{
              "workloadId": "{workload_id}",
              "vmName": "{vm_name}",
              "env": "work",
              "runtime": {MINIMAL_RUNTIME_JSON},
              "paths": {{
                "stateDir": "/var/lib/d2b/vms/{{vm}}",
                "runDir": "/run/d2b/vms/{{vm}}",
                "storeView": "/var/lib/d2b/vms/{{vm}}/store",
                "guestControlDir": "/run/d2b/vms/{{vm}}/gctl"
              }},
              "identity": {{
                "workloadId": "{workload_id}",
                "realmId": "{realm}",
                "realmPath": ["{realm}"],
                "canonicalTarget": "{workload_id}.{realm}.d2b",
                "legacyVmName": "{vm_name}"
              }}
            }}"#
        )
    }

    /// JSON for a workload WITHOUT a WorkloadIdentity (pre-W15 emitter).
    fn workload_no_identity(workload_id: &str, vm_name: &str) -> String {
        format!(
            r#"{{
              "workloadId": "{workload_id}",
              "vmName": "{vm_name}",
              "env": "work",
              "runtime": {MINIMAL_RUNTIME_JSON},
              "paths": {{
                "stateDir": "/var/lib/d2b/vms/{{vm}}",
                "runDir": "/run/d2b/vms/{{vm}}",
                "storeView": "/var/lib/d2b/vms/{{vm}}/store",
                "guestControlDir": "/run/d2b/vms/{{vm}}/gctl"
              }}
            }}"#
        )
    }

    const MINIMAL_RUNTIME_JSON: &str = r#"{
        "kind": "nixos",
        "provider": { "id": "local-ch", "driver": "cloud-hypervisor", "type": "local" },
        "capabilities": {
            "lifecycle": true, "display": false, "usbHotplug": false,
            "guestControl": true, "exec": true, "configSync": false,
            "ssh": false, "storeSync": true, "keys": false, "inGuestObservability": false
        },
        "operationCapabilities": {
            "lifecycle": { "start": true, "stop": true, "restart": true, "switch": true, "hostPrepare": false },
            "media": { "usbHotplug": false, "removableMedia": false, "qemuMedia": false },
            "display": { "display": false, "graphics": false, "video": false, "waylandProxy": false },
            "guest": { "guestControl": true, "exec": true, "shell": true, "configSync": false, "ssh": false, "keys": false, "inGuestObservability": false },
            "storage": { "storeSync": true, "virtiofs": true, "volumes": false }
        },
        "autostartPolicy": "host-boot-eligible"
    }"#;

    fn known_vms(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    // ------------------------------------------------------------------
    // Index construction
    // ------------------------------------------------------------------

    #[test]
    fn build_from_controllers_skips_workloads_without_identity() {
        let config = controllers_json_with_workloads(&format!(
            "[{}]",
            workload_no_identity("corp-vm", "corp-vm")
        ));
        let index = WorkloadTargetIndex::build_from_controllers(&config);
        assert!(index.is_empty());
    }

    #[test]
    fn build_from_controllers_indexes_identity_workloads() {
        let config = controllers_json_with_workloads(&format!(
            "[{}]",
            workload_with_identity("corp-vm", "work", "corp-vm")
        ));
        let index = WorkloadTargetIndex::build_from_controllers(&config);
        assert!(!index.is_empty());
        assert!(index.identity_for_vm("corp-vm").is_some());
    }

    #[test]
    fn identity_for_vm_returns_none_for_unknown_vm() {
        let config = controllers_json_no_workloads();
        let index = WorkloadTargetIndex::build_from_controllers(&config);
        assert!(index.identity_for_vm("unknown").is_none());
    }

    #[test]
    fn identity_for_vm_returns_identity_when_present() {
        let config = controllers_json_with_workloads(&format!(
            "[{}]",
            workload_with_identity("corp-vm", "work", "corp-vm")
        ));
        let index = WorkloadTargetIndex::build_from_controllers(&config);
        let found = index.identity_for_vm("corp-vm").expect("identity present");
        assert_eq!(found.canonical_target.to_canonical(), "corp-vm.work.d2b");
    }

    // ------------------------------------------------------------------
    // Target resolution — canonical target
    // ------------------------------------------------------------------

    #[test]
    fn resolve_canonical_target_succeeds() {
        let config = controllers_json_with_workloads(&format!(
            "[{}]",
            workload_with_identity("corp-vm", "work", "corp-vm")
        ));
        let index = WorkloadTargetIndex::build_from_controllers(&config);
        let result = index
            .resolve_target("corp-vm.work.d2b", &known_vms(&[]))
            .expect("canonical target resolves");
        assert_eq!(result.vm_name(), "corp-vm");
        assert!(matches!(
            result,
            TargetResolution::ResolvedFromCanonicalTarget { .. }
        ));
    }

    #[test]
    fn resolve_canonical_target_not_found_returns_error() {
        let config = controllers_json_no_workloads();
        let index = WorkloadTargetIndex::build_from_controllers(&config);
        let err = index
            .resolve_target("unknown.work.d2b", &known_vms(&[]))
            .expect_err("unknown canonical target is an error");
        assert!(matches!(err, TargetResolutionError::NotFound { .. }));
        assert!(err.message().contains("unknown.work.d2b"));
    }

    // ------------------------------------------------------------------
    // Target resolution — legacy VM name fast path
    // ------------------------------------------------------------------

    #[test]
    fn resolve_legacy_vm_name_passes_through_without_translation() {
        let config = controllers_json_no_workloads();
        let index = WorkloadTargetIndex::build_from_controllers(&config);
        let result = index
            .resolve_target("corp-vm", &known_vms(&["corp-vm"]))
            .expect("legacy VM name passes through");
        assert_eq!(result.vm_name(), "corp-vm");
        assert!(matches!(result, TargetResolution::LegacyVmName(_)));
    }

    // ------------------------------------------------------------------
    // Target resolution — bare identifiers never become realm aliases
    // ------------------------------------------------------------------

    #[test]
    fn bare_workload_id_is_not_resolved_as_an_alias() {
        let config = controllers_json_with_workloads(&format!(
            "[{}]",
            workload_with_identity("builder", "dev", "builder")
        ));
        let index = WorkloadTargetIndex::build_from_controllers(&config);
        // "builder" is not a known legacy VM name for the caller.
        let result = index
            .resolve_target("builder", &known_vms(&[]))
            .expect("bare identifier remains a legacy VM name");
        assert_eq!(result.vm_name(), "builder");
        assert!(matches!(result, TargetResolution::LegacyVmName(_)));
    }

    #[test]
    fn duplicate_workload_ids_do_not_create_alias_resolution() {
        let w1 = workload_with_identity("builder", "work", "builder-work");
        let w2 = workload_with_identity("builder", "dev", "builder-dev");
        // canonical targets already differ: builder.work.d2b vs builder.dev.d2b
        let config = controllers_json_with_workloads(&format!("[{w1}, {w2}]"));
        let index = WorkloadTargetIndex::build_from_controllers(&config);
        let result = index
            .resolve_target("builder", &known_vms(&[]))
            .expect("bare identifier remains a legacy VM name");
        assert!(matches!(result, TargetResolution::LegacyVmName(_)));
    }

    #[test]
    fn resolve_legacy_vm_name_remains_explicit() {
        let config = controllers_json_with_workloads(&format!(
            "[{}]",
            workload_with_identity("corp-vm", "work", "corp-vm")
        ));
        let index = WorkloadTargetIndex::build_from_controllers(&config);
        let result = index
            .resolve_target("corp-vm", &known_vms(&["corp-vm"]))
            .expect("legacy VM name takes priority");
        assert!(matches!(result, TargetResolution::LegacyVmName(_)));
        assert_eq!(result.vm_name(), "corp-vm");
    }

    #[test]
    fn resolve_unknown_target_falls_through_to_legacy_vm_name() {
        let config = controllers_json_no_workloads();
        let index = WorkloadTargetIndex::build_from_controllers(&config);
        // Not a .d2b target, not known, not in index — falls through.
        let result = index
            .resolve_target("nonexistent-vm", &known_vms(&[]))
            .expect("unknown target falls through as legacy VM name");
        assert!(matches!(result, TargetResolution::LegacyVmName(_)));
        assert_eq!(result.vm_name(), "nonexistent-vm");
    }

    #[test]
    fn alias_conflict_message_names_candidates() {
        let err = TargetResolutionError::AliasConflict {
            workload_id: "api".to_owned(),
            candidates: vec!["api-work".to_owned(), "api-dev".to_owned()],
        };
        let msg = err.message();
        assert!(msg.contains("api-work"));
        assert!(msg.contains("api-dev"));
        assert!(msg.contains("api.realm.d2b"));
    }

    // ------------------------------------------------------------------
    // Restart/adoption invariants — W16 requirement
    //
    // These tests prove the fundamental invariant the W13/W16 plan requires:
    // workload identity in the read model (list/status) is **config-driven**,
    // not state-driven. Daemon restart does not lose workload identity because:
    //
    //   1. Runner adoption uses (pid, start_time_ticks) from snapshot records
    //      in `supervisor/state.rs`; those records deliberately carry no
    //      workload identity — the process is adopted, not the identity.
    //   2. The `WorkloadTargetIndex` is rebuilt from `realm-controllers.json`
    //      on every public request, so as long as the config file is stable
    //      across restart, the workload identity returned by `identity_for_vm`
    //      is identical before and after restart.
    //
    // The tests below simulate the restart cycle by building the index twice
    // from the same config and asserting structural equality.
    // ------------------------------------------------------------------

    /// Restart simulation: building the index from the same config before and
    /// after restart produces identical `identity_for_vm` results for every
    /// declared workload.
    ///
    /// This proves the core restart/adoption invariant: workload identity is
    /// config-driven, so a daemon restart that reloads the config file returns
    /// the same identity without requiring it to be persisted in state.
    #[test]
    fn index_rebuilt_from_same_config_returns_identical_identity() {
        let config = controllers_json_with_workloads(&format!(
            "[{}, {}]",
            workload_with_identity("corp-vm", "work", "corp-vm"),
            workload_with_identity("dev-vm", "dev", "dev-vm"),
        ));

        // Simulate pre-restart daemon: build index once.
        let index_before = WorkloadTargetIndex::build_from_controllers(&config);

        // Simulate post-restart daemon: rebuild index from the same config.
        let index_after = WorkloadTargetIndex::build_from_controllers(&config);

        // Both indices must return the same identity for each workload — the
        // restart is a no-op for the read model.
        for vm in &["corp-vm", "dev-vm"] {
            let before = index_before
                .identity_for_vm(vm)
                .unwrap_or_else(|| panic!("identity missing pre-restart for {vm}"));
            let after = index_after
                .identity_for_vm(vm)
                .unwrap_or_else(|| panic!("identity missing post-restart for {vm}"));
            assert_eq!(
                before.canonical_target.to_canonical(),
                after.canonical_target.to_canonical(),
                "canonical_target diverged across restart simulation for {vm}"
            );
            assert_eq!(
                before.workload_id, after.workload_id,
                "workload_id diverged across restart simulation for {vm}"
            );
            assert_eq!(
                before.realm_id, after.realm_id,
                "realm_id diverged across restart simulation for {vm}"
            );
        }
    }

    /// Restart simulation: the index rebuilt from a JSON round-trip of the
    /// config (simulating the filesystem write/read that happens at restart)
    /// preserves every identity field without loss.
    #[test]
    fn index_rebuilt_after_config_json_round_trip_preserves_identity() {
        let config = controllers_json_with_workloads(&format!(
            "[{}]",
            workload_with_identity("corp-vm", "work", "corp-vm")
        ));
        let index_before = WorkloadTargetIndex::build_from_controllers(&config);
        let before_identity = index_before
            .identity_for_vm("corp-vm")
            .expect("identity present")
            .clone();

        // Round-trip the config through JSON (mirrors the filesystem serialization
        // the daemon performs before restart reads it back).
        let config_json = serde_json::to_string(&config).expect("serialize config");
        let config_reloaded: d2b_core::realm_controller_config::RealmControllersJson =
            serde_json::from_str(&config_json).expect("deserialize config");
        let index_after = WorkloadTargetIndex::build_from_controllers(&config_reloaded);
        let after_identity = index_after
            .identity_for_vm("corp-vm")
            .expect("identity present after json round-trip");

        assert_eq!(
            before_identity.canonical_target.to_canonical(),
            after_identity.canonical_target.to_canonical(),
            "canonical_target lost through config JSON round-trip"
        );
        assert_eq!(
            before_identity.workload_id, after_identity.workload_id,
            "workload_id lost through config JSON round-trip"
        );
        assert_eq!(
            before_identity.legacy_vm_name, after_identity.legacy_vm_name,
            "legacy_vm_name lost through config JSON round-trip"
        );
        assert_eq!(
            before_identity.realm_id, after_identity.realm_id,
            "realm_id lost through config JSON round-trip"
        );
        assert_eq!(
            before_identity.realm_path, after_identity.realm_path,
            "realm_path lost through config JSON round-trip"
        );
    }

    /// Restart simulation: a workload WITHOUT identity (transitional entry from
    /// a pre-W15 emitter) does NOT gain spurious identity after restart.
    /// The index is empty for such workloads both before and after a restart
    /// simulation, preserving backward-compat behavior.
    #[test]
    fn transitional_workload_without_identity_stays_absent_after_restart_simulation() {
        let config = controllers_json_with_workloads(&format!(
            "[{}]",
            workload_no_identity("corp-vm", "corp-vm")
        ));

        let index_before = WorkloadTargetIndex::build_from_controllers(&config);
        let index_after = WorkloadTargetIndex::build_from_controllers(&config);

        // Neither index must have an identity entry for the transitional workload.
        assert!(
            index_before.identity_for_vm("corp-vm").is_none(),
            "transitional workload must not have identity pre-restart"
        );
        assert!(
            index_after.identity_for_vm("corp-vm").is_none(),
            "transitional workload must not gain spurious identity post-restart"
        );
    }

    /// Mixed config: explicit workloads (with identity) and transitional
    /// workloads (without identity) both survive the restart cycle correctly —
    /// identity workloads return their identity, transitional ones remain None.
    #[test]
    fn mixed_config_restart_preserves_identity_only_for_explicit_workloads() {
        let config = controllers_json_with_workloads(&format!(
            "[{}, {}]",
            workload_with_identity("corp-vm", "work", "corp-vm"),
            workload_no_identity("legacy-vm", "legacy-vm"),
        ));

        let index_before = WorkloadTargetIndex::build_from_controllers(&config);
        let index_after = WorkloadTargetIndex::build_from_controllers(&config);

        // corp-vm has identity both before and after restart.
        assert!(
            index_before.identity_for_vm("corp-vm").is_some(),
            "corp-vm identity missing pre-restart"
        );
        assert!(
            index_after.identity_for_vm("corp-vm").is_some(),
            "corp-vm identity missing post-restart"
        );
        // legacy-vm has no identity either way.
        assert!(
            index_before.identity_for_vm("legacy-vm").is_none(),
            "legacy-vm must not have identity pre-restart"
        );
        assert!(
            index_after.identity_for_vm("legacy-vm").is_none(),
            "legacy-vm must not have identity post-restart"
        );
    }
}
