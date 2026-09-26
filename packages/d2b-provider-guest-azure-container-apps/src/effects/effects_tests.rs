    use d2b_contracts_resource::v3::ResourceRef;

    fn profile() -> AcaSandboxProfile {
        AcaSandboxProfile::new(
            AcaProfileId::parse("default").expect("profile id"),
            AcaDiskImageSource::ConfiguredDisk {
                binding_id: AcaConfiguredDiskId::parse("image-1").expect("disk id"),
            },
            AcaCpuMillis::new(500).expect("cpu"),
            AcaMemoryMib::new(2_048).expect("memory"),
            300,
            None,
        )
        .expect("profile")
    }

    fn runtime_config() -> AcaRuntimeConfig {
        AcaRuntimeConfig::new(
            profile(),
            AcaReadinessPolicy::new(3, 10).expect("readiness"),
            1_000,
            4,
        )
        .expect("runtime config")
    }

    fn provider_config() -> AcaProviderConfig {
        AcaProviderConfig::new(
            ResourceRef::parse("Guest/gateway").expect("gateway ref"),
            OpaqueAzureRef::parse("tenant").expect("tenant"),
            OpaqueAzureRef::parse("client").expect("client"),
            OpaqueAzureRef::parse("subscription").expect("subscription"),
            ResourceRef::parse("Credential/control").expect("control credential ref"),
            None,
            AcaConfiguredImageId::parse("environment").expect("environment id"),
            AcaConfiguredImageId::parse("resource-group").expect("resource group id"),
            None,
            AcaProfileId::parse("relay").expect("relay profile"),
            runtime_config(),
        )
        .expect("provider config")
    }

    #[test]
    fn runtime_config_new_enforces_plan_ttl_and_operation_capacity_bounds() {
        let readiness = AcaReadinessPolicy::new(3, 10).expect("readiness");
        for plan_ttl_ms in [0, MAX_ACA_PLAN_TTL_MS + 1] {
            assert_eq!(
                AcaRuntimeConfig::new(profile(), readiness, plan_ttl_ms, 4).err(),
                Some(AcaTypeError::InvalidPlanTtl),
                "plan_ttl_ms {plan_ttl_ms}"
            );
        }
        for capacity in [0, MAX_ACA_COMPLETED_OPERATIONS + 1] {
            assert_eq!(
                AcaRuntimeConfig::new(profile(), readiness, 1_000, capacity).err(),
                Some(AcaTypeError::InvalidOperationCapacity),
                "capacity {capacity}"
            );
        }
        assert!(AcaRuntimeConfig::new(profile(), readiness, 1, 1).is_ok());
        assert!(AcaRuntimeConfig::new(
            profile(),
            readiness,
            MAX_ACA_PLAN_TTL_MS,
            MAX_ACA_COMPLETED_OPERATIONS,
        )
        .is_ok());
    }

    #[test]
    fn sandbox_profile_new_enforces_auto_suspend_and_memory_bounds() {
        for auto_suspend_secs in [59, 86_401] {
            assert_eq!(
                AcaSandboxProfile::new(
                    AcaProfileId::parse("default").expect("profile id"),
                    AcaDiskImageSource::ConfiguredDisk {
                        binding_id: AcaConfiguredDiskId::parse("image-1").expect("disk id"),
                    },
                    AcaCpuMillis::new(500).expect("cpu"),
                    AcaMemoryMib::new(2_048).expect("memory"),
                    auto_suspend_secs,
                    None,
                )
                .err(),
                Some(AcaTypeError::InvalidResourceBounds),
                "auto_suspend_secs {auto_suspend_secs}"
            );
        }
        for memory_mib in [511, 640, 16_385] {
            assert_eq!(
                AcaMemoryMib::new(memory_mib).err(),
                Some(AcaTypeError::InvalidResourceBounds),
                "memory_mib {memory_mib}"
            );
        }
        assert!(AcaMemoryMib::new(512).is_ok());
        assert!(AcaMemoryMib::new(768).is_ok());
        assert!(AcaMemoryMib::new(16_384).is_ok());
        assert!(AcaSandboxProfile::new(
            AcaProfileId::parse("default").expect("profile id"),
            AcaDiskImageSource::ConfiguredDisk {
                binding_id: AcaConfiguredDiskId::parse("image-1").expect("disk id"),
            },
            AcaCpuMillis::new(500).expect("cpu"),
            AcaMemoryMib::new(2_048).expect("memory"),
            60,
            None,
        )
        .is_ok());
        assert!(AcaSandboxProfile::new(
            AcaProfileId::parse("default").expect("profile id"),
            AcaDiskImageSource::ConfiguredDisk {
                binding_id: AcaConfiguredDiskId::parse("image-1").expect("disk id"),
            },
            AcaCpuMillis::new(500).expect("cpu"),
            AcaMemoryMib::new(2_048).expect("memory"),
            86_400,
            None,
        )
        .is_ok());
    }

    #[test]
    fn provider_config_new_enforces_resource_type_checks() {
        let valid = provider_config();
        assert!(valid.validate().is_ok());

        let mis_typed = |field: &str| {
            let config = AcaProviderConfig::new(
                if field == "gateway" {
                    ResourceRef::parse("Process/not-a-guest").expect("ref")
                } else {
                    ResourceRef::parse("Guest/gateway").expect("gateway ref")
                },
                OpaqueAzureRef::parse("tenant").expect("tenant"),
                OpaqueAzureRef::parse("client").expect("client"),
                OpaqueAzureRef::parse("subscription").expect("subscription"),
                if field == "control" {
                    ResourceRef::parse("Process/not-a-credential").expect("ref")
                } else {
                    ResourceRef::parse("Credential/control").expect("control credential ref")
                },
                if field == "pull" {
                    Some(ResourceRef::parse("Process/not-a-credential").expect("ref"))
                } else {
                    None
                },
                AcaConfiguredImageId::parse("environment").expect("environment id"),
                AcaConfiguredImageId::parse("resource-group").expect("resource group id"),
                if field == "network" {
                    Some(ResourceRef::parse("Process/not-a-network").expect("ref"))
                } else {
                    None
                },
                AcaProfileId::parse("relay").expect("relay profile"),
                runtime_config(),
            )
            .err();
            assert_eq!(config, Some(AcaTypeError::InvalidExecutionBoundary), "{field}");
        };
        for field in ["gateway", "control", "pull", "network"] {
            mis_typed(field);
        }
    }

    #[test]
    fn runtime_config_deserialization_revalidates_constructor_bounds() {

