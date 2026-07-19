use d2b_core::realm_controller_config::{
    RealmControllerConfig, RealmControllerPlacement as MetadataPlacement, RealmControllersJson,
};
use d2b_realm_core::{
    Capability, CapabilityPreflightDenialReason, CapabilityPreflightStatus, CapabilitySet,
    ControllerGenerationId, HostLocalPeerCredentialSemantics, ProtocolToken,
    RealmAccessAliasSource, RealmAccessBinding, RealmAccessCapabilityPreflight,
    RealmAccessClientBinding, RealmAccessClientBindingKind, RealmAccessClientContract,
    RealmAccessResolverDiagnostic, RealmAccessResolverError, RealmAccessResolverRequest,
    RealmAccessResolverResponse, RealmControllerPlacement, RealmId, RealmPath, RealmTarget,
    RealmTransportBinding, UnixSocketPath,
};
use sha2::{Digest, Sha256};

use crate::LoadedRealmControllersConfig;

pub fn resolve_local_root_realm_access(
    loaded: Option<&LoadedRealmControllersConfig>,
    request: &RealmAccessResolverRequest,
    expected_controller_generation: Option<&ControllerGenerationId>,
) -> Result<RealmAccessResolverResponse, RealmAccessResolverError> {
    let resolved = resolve_requested_target(request)?;
    let Some(loaded) = loaded else {
        return Err(missing_controller_error(resolved.target.realm.clone()));
    };

    let observed_generation = realm_controllers_config_generation(&loaded.config);
    if let Some(expected_generation) = expected_controller_generation
        && expected_generation != &observed_generation
    {
        return Err(stale_controller_error(
            resolved.target.realm.clone(),
            expected_generation.clone(),
            Some(observed_generation),
        ));
    }

    let Some(controller) = find_controller_for_realm(&loaded.config, &resolved.target.realm) else {
        return Err(missing_controller_error(resolved.target.realm.clone()));
    };
    if !matches!(controller.placement, MetadataPlacement::HostLocal) {
        return Err(missing_binding_error(&resolved.target));
    }
    if !client_supports_direct_host_local(&request.client) {
        return Err(missing_binding_error(&resolved.target));
    }

    let Some(socket_path) = UnixSocketPath::parse(controller.sockets.public_socket_path.as_str())
    else {
        return Err(missing_binding_error(&resolved.target));
    };
    let transport = RealmTransportBinding::LocalUnixSocket { socket_path };
    let client_binding = RealmAccessClientBinding::from_transport(&transport);
    let access_binding = RealmAccessBinding {
        realm: resolved.target.realm.clone(),
        controller_generation: observed_generation,
        placement: RealmControllerPlacement::HostLocal,
        transport,
    };
    let advertised_capabilities = advertised_capabilities_from_controller(controller);
    let capability_preflight =
        host_local_capability_preflight(&request.required_capabilities, &advertised_capabilities);

    Ok(RealmAccessResolverResponse {
        canonical_target: resolved.target,
        resolved_realm: access_binding.realm.clone(),
        placement: access_binding.placement.clone(),
        client_binding,
        access_binding,
        capability_preflight,
        alias_source: resolved.alias_source,
        default_realm: None,
        diagnostics: Vec::new(),
    })
}

pub fn local_root_realm_access_client_contract() -> RealmAccessClientContract {
    RealmAccessClientContract {
        supported_bindings: vec![RealmAccessClientBindingKind::DirectHostLocalUnixSocket],
        require_direct_local_so_peercred: true,
    }
}

pub fn host_local_capability_preflight_placeholder(
    required: &CapabilitySet,
) -> RealmAccessCapabilityPreflight {
    RealmAccessCapabilityPreflight {
        required: required.clone(),
        advertised: required.clone(),
        status: CapabilityPreflightStatus::Satisfied,
    }
}

pub fn advertised_capabilities_from_controller(
    controller: &RealmControllerConfig,
) -> CapabilitySet {
    let provider_refs = CapabilitySet::from_tokens(
        controller
            .providers
            .iter()
            .filter(|provider| provider.enabled)
            .flat_map(|provider| provider.capability_refs.iter())
            .filter_map(|capability_ref| ProtocolToken::parse(capability_ref.clone()).ok()),
    );
    let mut advertised = provider_refs;
    for capability in local_runtime_capabilities_from_controller(controller).iter() {
        advertised = advertised.with(capability);
    }
    advertised
}

fn local_runtime_capabilities_from_controller(controller: &RealmControllerConfig) -> CapabilitySet {
    let Some(local_runtime) = &controller.local_runtime else {
        return CapabilitySet::empty();
    };

    let mut advertised = CapabilitySet::empty();
    for workload in &local_runtime.workloads {
        let ops = &workload.runtime.operation_capabilities;
        if ops.lifecycle.start
            || ops.lifecycle.stop
            || ops.lifecycle.restart
            || ops.lifecycle.switch
            || ops.lifecycle.host_prepare
        {
            advertised = advertised.with(Capability::Lifecycle);
        }
        if ops.guest.exec {
            advertised = advertised.with(Capability::Exec);
        }
        if ops.guest.shell {
            advertised = advertised.with(Capability::PersistentShell);
        }
        if ops.guest.guest_control {
            advertised = advertised.with(Capability::Vsock);
        }
        if ops.display.display || ops.display.wayland_proxy {
            advertised = advertised.with(Capability::WindowForwarding);
        }
        if ops.display.graphics {
            advertised = advertised.with(Capability::GpuAccel);
        }
        if ops.media.usb_hotplug {
            advertised = advertised.with(Capability::Usb).with(Capability::Hotplug);
        }
        if ops.media.removable_media || ops.media.qemu_media {
            advertised = advertised.with(Capability::Hotplug);
        }
        if ops.storage.store_sync || ops.storage.virtiofs {
            advertised = advertised.with(Capability::Virtiofs);
        }
    }
    advertised
}

pub fn host_local_capability_preflight(
    required: &CapabilitySet,
    advertised: &CapabilitySet,
) -> RealmAccessCapabilityPreflight {
    let status = if required.is_subset_of(advertised) {
        CapabilityPreflightStatus::Satisfied
    } else {
        CapabilityPreflightStatus::Denied {
            reason: CapabilityPreflightDenialReason::MissingCapability,
            missing: missing_capabilities(required, advertised),
        }
    };

    RealmAccessCapabilityPreflight {
        required: required.clone(),
        advertised: advertised.clone(),
        status,
    }
}

fn missing_capabilities(required: &CapabilitySet, advertised: &CapabilitySet) -> Vec<Capability> {
    required
        .iter()
        .filter(|capability| !advertised.has(*capability))
        .collect()
}

pub fn direct_host_local_peercred_semantics() -> HostLocalPeerCredentialSemantics {
    HostLocalPeerCredentialSemantics::direct_client_peercred()
}

pub fn realm_controllers_config_generation(
    config: &RealmControllersJson,
) -> ControllerGenerationId {
    let bytes = serde_json::to_vec(config).expect("realm controller metadata is serializable");
    let hash = Sha256::digest(bytes);
    let mut suffix = String::with_capacity(16);
    for byte in hash.iter().take(8) {
        use std::fmt::Write as _;
        write!(&mut suffix, "{byte:02x}").expect("write to string");
    }
    ControllerGenerationId::parse(format!("metadata-{suffix}"))
        .expect("generated controller generation id is valid")
}

struct ResolvedTarget {
    target: RealmTarget,
    alias_source: RealmAccessAliasSource,
}

fn resolve_requested_target(
    request: &RealmAccessResolverRequest,
) -> Result<ResolvedTarget, RealmAccessResolverError> {
    RealmTarget::parse(request.requested_target.as_str())
        .map(|target| ResolvedTarget {
            target,
            alias_source: RealmAccessAliasSource::FullyQualified,
        })
        .map_err(|_| missing_controller_error(RealmPath::local()))
}

fn find_controller_for_realm<'a>(
    config: &'a RealmControllersJson,
    realm: &RealmPath,
) -> Option<&'a RealmControllerConfig> {
    config.controllers.iter().find(|controller| {
        realm_path_from_metadata(controller.realm_path.as_str()).as_ref() == Some(realm)
    })
}

fn realm_path_from_metadata(raw: &str) -> Option<RealmPath> {
    let labels = raw
        .split('.')
        .map(RealmId::parse)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    RealmPath::new(labels)
}

fn client_supports_direct_host_local(client: &RealmAccessClientContract) -> bool {
    client
        .supported_bindings
        .contains(&RealmAccessClientBindingKind::DirectHostLocalUnixSocket)
}

fn missing_controller_error(realm: RealmPath) -> RealmAccessResolverError {
    RealmAccessResolverError {
        diagnostic: RealmAccessResolverDiagnostic::MissingRealmController { realm },
        related: Vec::new(),
    }
}

fn stale_controller_error(
    realm: RealmPath,
    expected_generation: ControllerGenerationId,
    observed_generation: Option<ControllerGenerationId>,
) -> RealmAccessResolverError {
    RealmAccessResolverError {
        diagnostic: RealmAccessResolverDiagnostic::StaleRealmController {
            realm,
            expected_generation,
            observed_generation,
        },
        related: Vec::new(),
    }
}

fn missing_binding_error(target: &RealmTarget) -> RealmAccessResolverError {
    RealmAccessResolverError {
        diagnostic: RealmAccessResolverDiagnostic::MissingRealmBinding {
            target: target.clone(),
            realm: target.realm.clone(),
        },
        related: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_core::realm_controller_config::RealmControllerMetadataSummary;
    use d2b_realm_core::{
        Capability, HostLocalPeerCredentialChecker, HostLocalPeerCredentialSource,
        HostLocalProxyStatus, RealmAccessTargetInput,
    };

    fn loaded_controller(public_socket_path: &str) -> LoadedRealmControllersConfig {
        loaded_controller_with_placement("host-local", public_socket_path)
    }

    fn loaded_controller_with_placement(
        placement: &str,
        public_socket_path: &str,
    ) -> LoadedRealmControllersConfig {
        loaded_controller_with_placement_and_capability_refs(
            placement,
            public_socket_path,
            ["lifecycle"],
            "",
        )
    }

    fn loaded_controller_with_capability_refs<I, S>(
        capability_refs: I,
    ) -> LoadedRealmControllersConfig
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        loaded_controller_with_placement_and_capability_refs(
            "host-local",
            "/run/d2b/realms/work/public.sock",
            capability_refs,
            "",
        )
    }

    fn loaded_controller_with_placement_and_capability_refs<I, S>(
        placement: &str,
        public_socket_path: &str,
        capability_refs: I,
        local_runtime_json: &str,
    ) -> LoadedRealmControllersConfig
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let capability_refs = capability_refs
            .into_iter()
            .map(|capability_ref| capability_ref.as_ref().to_owned())
            .collect::<Vec<_>>();
        let capability_refs_json =
            serde_json::to_string(&capability_refs).expect("capability refs serialize");
        let raw = format!(
            r#"{{
              "schemaVersion": "v2",
              "runtimeState": "metadata-only",
              "controllers": [
                {{
                  "realmName": "Work",
                  "realmId": "work",
                  "realmPath": "work",
                  "placement": "{placement}",
                  "daemon": {{
                    "user": "d2br-0123456789abcdef",
                    "group": "d2br-0123456789abcdef",
                    "publicSocketGroup": "d2br-0123456789abcdef",
                    "serviceName": "d2b-realm-work-daemon.service",
                    "configPath": "/etc/d2b/realms/work/daemon-config.json",
                    "stateLockPath": "/run/d2b/realms/work/daemon.lock",
                    "locksDir": "/run/d2b/realms/work/locks",
                    "socketActivated": false,
                    "materializedService": false
                  }},
                  "broker": {{
                    "enabled": true,
                    "hostMutation": false,
                    "user": "root",
                    "group": "d2br-0123456789abcdef",
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
                    "publicSocketPath": "{public_socket_path}",
                    "brokerSocketPath": "/run/d2b/realms/work/priv.sock"
                  }},
                  "allocator": {{
                    "kind": "local-root-metadata",
                    "configPath": "/etc/d2b/allocator.json",
                    "rootSocket": "/run/d2b/allocator.sock"
                  }},
                  "access": {{
                    "allowedUsers": ["alice"],
                    "allowedGroups": ["d2b"],
                    "inheritedAdminUsers": ["admin"]
                  }},
                  {local_runtime_json}
                  "providers": [
                    {{
                      "providerName": "local",
                      "providerId": "local",
                      "enabled": true,
                      "kind": "host-local",
                      "placement": "{placement}",
                      "capabilityRefs": {capability_refs_json}
                    }}
                  ]
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
        let config: RealmControllersJson =
            serde_json::from_str(&raw).expect("realm controller fixture parses");
        let summary: RealmControllerMetadataSummary =
            config.validate_metadata_only().expect("metadata validates");
        LoadedRealmControllersConfig { config, summary }
    }

    fn loaded_controller_with_local_runtime() -> LoadedRealmControllersConfig {
        loaded_controller_with_placement_and_capability_refs(
            "host-local",
            "/run/d2b/realms/work/public.sock",
            Vec::<String>::new(),
            r#""localRuntime": {
                    "runtimeState": "metadata-only",
                    "providers": [
                      {
                        "kind": "nixos",
                        "provider": {
                          "id": "local-cloud-hypervisor",
                          "driver": "cloud-hypervisor",
                          "type": "local"
                        },
                        "capabilities": {
                          "lifecycle": true,
                          "display": true,
                          "usbHotplug": true,
                          "guestControl": true,
                          "exec": true,
                          "configSync": true,
                          "ssh": true,
                          "storeSync": true,
                          "keys": true,
                          "inGuestObservability": true
                        },
                        "operationCapabilities": {
                          "lifecycle": {
                            "start": true,
                            "stop": true,
                            "restart": true,
                            "switch": true,
                            "hostPrepare": true
                          },
                          "media": {
                            "usbHotplug": true,
                            "removableMedia": false,
                            "qemuMedia": false
                          },
                          "display": {
                            "display": true,
                            "graphics": true,
                            "video": true,
                            "waylandProxy": true
                          },
                          "guest": {
                            "guestControl": true,
                            "exec": true,
                            "shell": true,
                            "configSync": true,
                            "ssh": true,
                            "keys": true,
                            "inGuestObservability": true
                          },
                          "storage": {
                            "storeSync": true,
                            "virtiofs": true,
                            "volumes": true
                          }
                        },
                        "autostartPolicy": "host-boot-eligible",
                        "services": []
                      }
                    ],
                    "workloads": [
                      {
                        "workloadId": "builder",
                        "vmName": "builder",
                        "env": "work",
                        "runtime": {
                          "kind": "nixos",
                          "provider": {
                            "id": "local-cloud-hypervisor",
                            "driver": "cloud-hypervisor",
                            "type": "local"
                          },
                          "capabilities": {
                            "lifecycle": true,
                            "display": true,
                            "usbHotplug": true,
                            "guestControl": true,
                            "exec": true,
                            "configSync": true,
                            "ssh": true,
                            "storeSync": true,
                            "keys": true,
                            "inGuestObservability": true
                          },
                          "operationCapabilities": {
                            "lifecycle": {
                              "start": true,
                              "stop": true,
                              "restart": true,
                              "switch": true,
                              "hostPrepare": true
                            },
                            "media": {
                              "usbHotplug": true,
                              "removableMedia": false,
                              "qemuMedia": false
                            },
                            "display": {
                              "display": true,
                              "graphics": true,
                              "video": true,
                              "waylandProxy": true
                            },
                            "guest": {
                              "guestControl": true,
                              "exec": true,
                              "shell": true,
                              "configSync": true,
                              "ssh": true,
                              "keys": true,
                              "inGuestObservability": true
                            },
                            "storage": {
                              "storeSync": true,
                              "virtiofs": true,
                              "volumes": true
                            }
                          },
                          "autostartPolicy": "host-boot-eligible",
                          "services": []
                        },
                        "paths": {
                          "stateDir": "/var/lib/d2b/vms/builder",
                          "runDir": "/run/d2b/vms/builder",
                          "storeView": "/var/lib/d2b/vms/builder/store-view",
                          "guestControlDir": "/run/d2b/vms/builder/guest-control"
                        }
                      }
                    ],
                    "invariants": {
                      "metadataOnly": true,
                      "existingGlobalVmPathsPreserved": true,
                      "noStateMigrationDuringActivation": true,
                      "brokerEffectsRemainRealmDelegated": true
                    }
                  },"#,
        )
    }

    fn request(target: &str) -> RealmAccessResolverRequest {
        RealmAccessResolverRequest {
            requested_target: RealmAccessTargetInput::parse(target).expect("target input"),
            default_realm: None,
            aliases: Vec::new(),
            required_capabilities: CapabilitySet::from_caps([Capability::Lifecycle]),
            client: local_root_realm_access_client_contract(),
        }
    }

    fn realm(label: &str) -> RealmPath {
        RealmPath::new(vec![RealmId::parse(label).expect("realm id")]).expect("realm path")
    }

    #[test]
    fn resolves_host_local_controller_to_direct_unix_binding() {
        let loaded = loaded_controller("/run/d2b/realms/work/public.sock");
        let generation = realm_controllers_config_generation(&loaded.config);
        let response = resolve_local_root_realm_access(
            Some(&loaded),
            &request("builder.work.d2b"),
            Some(&generation),
        )
        .expect("direct host-local binding");

        assert_eq!(response.resolved_realm, realm("work"));
        assert_eq!(response.placement, RealmControllerPlacement::HostLocal);
        assert_eq!(
            response.capability_preflight,
            host_local_capability_preflight(
                &CapabilitySet::from_caps([Capability::Lifecycle]),
                &CapabilitySet::from_caps([Capability::Lifecycle])
            )
        );
        match &response.access_binding.transport {
            RealmTransportBinding::LocalUnixSocket { socket_path } => {
                assert_eq!(socket_path.as_str(), "/run/d2b/realms/work/public.sock");
            }
            other => panic!("expected direct local Unix binding, got {other:?}"),
        }
        match &response.client_binding {
            RealmAccessClientBinding::DirectHostLocalUnix {
                socket_path,
                peer_credentials,
            } => {
                assert_eq!(socket_path.as_str(), "/run/d2b/realms/work/public.sock");
                assert_eq!(*peer_credentials, direct_host_local_peercred_semantics());
            }
            other => panic!("expected direct client binding, got {other:?}"),
        }
    }

    #[test]
    fn missing_and_stale_realm_controllers_fail_closed() {
        let missing = resolve_local_root_realm_access(None, &request("builder.work.d2b"), None)
            .expect_err("missing controller metadata fails closed");
        assert_eq!(
            missing.diagnostic,
            RealmAccessResolverDiagnostic::MissingRealmController {
                realm: realm("work")
            }
        );

        let loaded = loaded_controller("/run/d2b/realms/work/public.sock");
        let stale = ControllerGenerationId::parse("gen-stale").expect("stale generation");
        let error = resolve_local_root_realm_access(
            Some(&loaded),
            &request("builder.work.d2b"),
            Some(&stale),
        )
        .expect_err("stale controller metadata fails closed");
        match error.diagnostic {
            RealmAccessResolverDiagnostic::StaleRealmController {
                realm,
                expected_generation,
                observed_generation,
            } => {
                assert_eq!(realm, realm_path_from_metadata("work").unwrap());
                assert_eq!(expected_generation, stale);
                assert_eq!(
                    observed_generation,
                    Some(realm_controllers_config_generation(&loaded.config))
                );
            }
            other => panic!("expected stale controller diagnostic, got {other:?}"),
        }
    }

    #[test]
    fn capability_preflight_placeholder_is_only_for_empty_required_capabilities() {
        let required = CapabilitySet::empty();
        let preflight = host_local_capability_preflight_placeholder(&required);

        assert_eq!(preflight.required, required);
        assert_eq!(preflight.advertised, required);
        assert_eq!(preflight.status, CapabilityPreflightStatus::Satisfied);
    }

    #[test]
    fn capability_preflight_is_satisfied_by_advertised_provider_refs() {
        let loaded = loaded_controller_with_capability_refs(["lifecycle", "exec"]);
        let mut req = request("builder.work.d2b");
        req.required_capabilities =
            CapabilitySet::from_caps([Capability::Lifecycle, Capability::Exec]);

        let response = resolve_local_root_realm_access(Some(&loaded), &req, None)
            .expect("capabilities advertised by provider refs");

        assert_eq!(
            response.capability_preflight.required,
            req.required_capabilities
        );
        assert_eq!(
            response.capability_preflight.advertised,
            CapabilitySet::from_caps([Capability::Lifecycle, Capability::Exec])
        );
        assert_eq!(
            response.capability_preflight.status,
            CapabilityPreflightStatus::Satisfied
        );
    }

    #[test]
    fn missing_required_capabilities_are_denied_fail_closed() {
        let loaded = loaded_controller_with_capability_refs(["lifecycle"]);
        let mut req = request("builder.work.d2b");
        req.required_capabilities =
            CapabilitySet::from_caps([Capability::Lifecycle, Capability::Exec, Capability::Logs]);

        let response = resolve_local_root_realm_access(Some(&loaded), &req, None)
            .expect("binding resolves but capability preflight denies");

        assert_eq!(
            response.capability_preflight.advertised,
            CapabilitySet::from_caps([Capability::Lifecycle])
        );
        assert_eq!(
            response.capability_preflight.status,
            CapabilityPreflightStatus::Denied {
                reason: CapabilityPreflightDenialReason::MissingCapability,
                missing: vec![Capability::Exec, Capability::Logs],
            }
        );
    }

    #[test]
    fn local_runtime_metadata_contributes_to_host_local_capability_preflight() {
        let loaded = loaded_controller_with_local_runtime();
        let mut req = request("builder.work.d2b");
        req.required_capabilities = CapabilitySet::from_caps([
            Capability::Lifecycle,
            Capability::Exec,
            Capability::PersistentShell,
            Capability::Vsock,
            Capability::WindowForwarding,
            Capability::GpuAccel,
            Capability::Usb,
            Capability::Hotplug,
            Capability::Virtiofs,
        ]);

        let response = resolve_local_root_realm_access(Some(&loaded), &req, None)
            .expect("local runtime metadata advertises required capabilities");

        assert_eq!(
            response.capability_preflight.required,
            req.required_capabilities
        );
        assert!(
            response
                .capability_preflight
                .advertised
                .has(Capability::Lifecycle)
        );
        assert!(
            response
                .capability_preflight
                .advertised
                .has(Capability::Exec)
        );
        assert!(
            response
                .capability_preflight
                .advertised
                .has(Capability::PersistentShell)
        );
        assert!(
            response
                .capability_preflight
                .advertised
                .has(Capability::Vsock)
        );
        assert!(
            response
                .capability_preflight
                .advertised
                .has(Capability::WindowForwarding)
        );
        assert!(
            response
                .capability_preflight
                .advertised
                .has(Capability::GpuAccel)
        );
        assert!(
            response
                .capability_preflight
                .advertised
                .has(Capability::Usb)
        );
        assert!(
            response
                .capability_preflight
                .advertised
                .has(Capability::Hotplug)
        );
        assert!(
            response
                .capability_preflight
                .advertised
                .has(Capability::Virtiofs)
        );
        assert_eq!(
            response.capability_preflight.status,
            CapabilityPreflightStatus::Satisfied
        );
    }

    #[test]
    fn empty_required_capabilities_still_report_advertised_capabilities() {
        let loaded = loaded_controller_with_capability_refs(["lifecycle", "exec"]);
        let mut req = request("builder.work.d2b");
        req.required_capabilities = CapabilitySet::empty();

        let response = resolve_local_root_realm_access(Some(&loaded), &req, None)
            .expect("empty required capabilities need no advertisement");

        assert_eq!(
            response.capability_preflight.required,
            CapabilitySet::empty()
        );
        assert_eq!(
            response.capability_preflight.advertised,
            CapabilitySet::from_caps([Capability::Lifecycle, Capability::Exec])
        );
        assert_eq!(
            response.capability_preflight.status,
            CapabilityPreflightStatus::Satisfied
        );
    }

    #[test]
    fn direct_binding_represents_peercred_and_fd_passing_semantics() {
        let loaded = loaded_controller("/run/d2b/realms/work/public.sock");
        let response =
            resolve_local_root_realm_access(Some(&loaded), &request("builder.work.d2b"), None)
                .expect("direct binding");

        let RealmAccessClientBinding::DirectHostLocalUnix {
            peer_credentials, ..
        } = response.client_binding
        else {
            panic!("host-local resolution must not return a proxy or remote transport");
        };
        assert_eq!(
            peer_credentials.source,
            HostLocalPeerCredentialSource::ConnectingClientProcess
        );
        assert_eq!(
            peer_credentials.checked_by,
            HostLocalPeerCredentialChecker::D2bdPublicSocket
        );
        assert_eq!(peer_credentials.proxy, HostLocalProxyStatus::NoByteProxy);
    }

    #[test]
    fn unsupported_client_contract_does_not_fall_back_to_byte_proxy() {
        let loaded = loaded_controller("/run/d2b/realms/work/public.sock");
        let mut req = request("builder.work.d2b");
        req.client.supported_bindings = vec![RealmAccessClientBindingKind::RemoteRealmTransportRef];

        let error = resolve_local_root_realm_access(Some(&loaded), &req, None)
            .expect_err("no byte-proxy fallback");

        assert!(matches!(
            error.diagnostic,
            RealmAccessResolverDiagnostic::MissingRealmBinding { .. }
        ));
    }

    #[test]
    fn non_host_local_or_invalid_socket_metadata_fails_without_proxy_fallback() {
        let gateway_placement =
            loaded_controller_with_placement("gateway-vm", "/run/d2b/realms/work/public.sock");
        let error = resolve_local_root_realm_access(
            Some(&gateway_placement),
            &request("builder.work.d2b"),
            None,
        )
        .expect_err("local root resolver only consumes direct host-local metadata");
        assert!(matches!(
            error.diagnostic,
            RealmAccessResolverDiagnostic::MissingRealmBinding { .. }
        ));

        let invalid_socket = loaded_controller("/run/d2b/realms/../work/public.sock");
        let error = resolve_local_root_realm_access(
            Some(&invalid_socket),
            &request("builder.work.d2b"),
            None,
        )
        .expect_err("invalid Unix socket metadata fails closed");
        assert!(matches!(
            error.diagnostic,
            RealmAccessResolverDiagnostic::MissingRealmBinding { .. }
        ));
    }

    #[test]
    fn missing_realm_row_is_reported_as_missing_controller() {
        let loaded = loaded_controller("/run/d2b/realms/work/public.sock");
        let error =
            resolve_local_root_realm_access(Some(&loaded), &request("builder.dev.d2b"), None)
                .expect_err("unlisted realm fails closed");

        assert_eq!(
            error.diagnostic,
            RealmAccessResolverDiagnostic::MissingRealmController {
                realm: realm("dev")
            }
        );
    }

    #[test]
    fn bare_targets_fail_closed() {
        let loaded = loaded_controller("/run/d2b/realms/work/public.sock");
        let req = request("builder");
        let error = resolve_local_root_realm_access(Some(&loaded), &req, None)
            .expect_err("bare target fails closed");
        assert!(matches!(
            error.diagnostic,
            RealmAccessResolverDiagnostic::MissingRealmController { .. }
        ));
    }
}
