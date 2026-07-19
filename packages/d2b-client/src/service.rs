use std::fmt;

use d2b_contracts::v2_services::{
    MethodSpec, SERVICE_INVENTORY, ServiceSpec, activation_ttrpc, broker_ttrpc,
    clipboard_picker_ttrpc, clipboard_ttrpc, daemon_ttrpc, guest_ttrpc, notify_ttrpc,
    provider_audio_ttrpc, provider_credential_ttrpc, provider_device_ttrpc, provider_display_ttrpc,
    provider_infrastructure_ttrpc, provider_network_ttrpc, provider_observability_ttrpc,
    provider_runtime_ttrpc, provider_storage_ttrpc, provider_substrate_ttrpc,
    provider_transport_ttrpc, realm_ttrpc, runtime_systemd_user_ttrpc, security_key_ttrpc,
    shell_ttrpc, tty_ttrpc, user_ttrpc, wayland_ttrpc,
};

use crate::ClientError;

macro_rules! services {
    (
        $(
            $variant:ident, $accessor:ident, $module:ident, $client:ident;
        )+
    ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u8)]
        pub enum ServiceKind {
            $($variant,)+
        }

        impl ServiceKind {
            pub const ALL: &'static [Self] = &[$(Self::$variant,)+];

            pub fn spec(self) -> &'static ServiceSpec {
                &SERVICE_INVENTORY[self as usize]
            }
        }

        #[derive(Clone)]
        pub enum GeneratedClient {
            $($variant($module::$client),)+
        }

        impl GeneratedClient {
            fn new(kind: ServiceKind, client: ttrpc::r#async::Client) -> Self {
                match kind {
                    $(ServiceKind::$variant => Self::$variant($module::$client::new(client)),)+
                }
            }

            pub const fn kind(&self) -> ServiceKind {
                match self {
                    $(Self::$variant(_) => ServiceKind::$variant,)+
                }
            }

            $(
                pub fn $accessor(&self) -> Result<&$module::$client, ClientError> {
                    match self {
                        Self::$variant(client) => Ok(client),
                        _ => Err(ClientError::InvalidService),
                    }
                }
            )+
        }
    };
}

services! {
    Daemon, daemon, daemon_ttrpc, DaemonServiceClient;
    Realm, realm, realm_ttrpc, RealmServiceClient;
    Guest, guest, guest_ttrpc, GuestServiceClient;
    ProviderRuntime, provider_runtime, provider_runtime_ttrpc, RuntimeProviderServiceClient;
    ProviderInfrastructure, provider_infrastructure, provider_infrastructure_ttrpc, InfrastructureProviderServiceClient;
    ProviderTransport, provider_transport, provider_transport_ttrpc, TransportProviderServiceClient;
    ProviderSubstrate, provider_substrate, provider_substrate_ttrpc, SubstrateProviderServiceClient;
    ProviderCredential, provider_credential, provider_credential_ttrpc, CredentialProviderServiceClient;
    ProviderDisplay, provider_display, provider_display_ttrpc, DisplayProviderServiceClient;
    ProviderNetwork, provider_network, provider_network_ttrpc, NetworkProviderServiceClient;
    ProviderStorage, provider_storage, provider_storage_ttrpc, StorageProviderServiceClient;
    ProviderDevice, provider_device, provider_device_ttrpc, DeviceProviderServiceClient;
    ProviderAudio, provider_audio, provider_audio_ttrpc, AudioProviderServiceClient;
    ProviderObservability, provider_observability, provider_observability_ttrpc, ObservabilityProviderServiceClient;
    Broker, broker, broker_ttrpc, BrokerServiceClient;
    User, user, user_ttrpc, UserServiceClient;
    RuntimeSystemdUser, runtime_systemd_user, runtime_systemd_user_ttrpc, RuntimeSystemdUserServiceClient;
    Shell, shell, shell_ttrpc, ShellServiceClient;
    Clipboard, clipboard, clipboard_ttrpc, ClipboardServiceClient;
    ClipboardPicker, clipboard_picker, clipboard_picker_ttrpc, ClipboardPickerServiceClient;
    Notify, notify, notify_ttrpc, NotifyServiceClient;
    SecurityKey, security_key, security_key_ttrpc, SecurityKeyServiceClient;
    Wayland, wayland, wayland_ttrpc, WaylandServiceClient;
    Activation, activation, activation_ttrpc, ActivationServiceClient;
    Tty, tty, tty_ttrpc, TtyServiceClient;
}

impl fmt::Debug for GeneratedClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GeneratedClient")
            .field("service", &self.kind())
            .finish()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MethodHandle {
    service: ServiceKind,
    index: u16,
    spec: &'static MethodSpec,
}

impl MethodHandle {
    pub const fn service(self) -> ServiceKind {
        self.service
    }

    pub const fn index(self) -> u16 {
        self.index
    }

    pub const fn spec(self) -> &'static MethodSpec {
        self.spec
    }
}

#[derive(Clone)]
pub struct ServiceHandle {
    kind: ServiceKind,
    generated: GeneratedClient,
    raw: ttrpc::r#async::Client,
}

impl ServiceHandle {
    pub(crate) fn new(kind: ServiceKind, client: ttrpc::r#async::Client) -> Self {
        Self {
            kind,
            generated: GeneratedClient::new(kind, client.clone()),
            raw: client,
        }
    }

    pub const fn kind(&self) -> ServiceKind {
        self.kind
    }

    pub fn generated(&self) -> &GeneratedClient {
        &self.generated
    }

    pub(crate) fn proxy(&self, kind: ServiceKind) -> Self {
        Self::new(kind, self.raw.clone())
    }

    pub fn method(&self, index: u16) -> Result<MethodHandle, ClientError> {
        let spec = self
            .kind
            .spec()
            .methods
            .get(usize::from(index))
            .ok_or(ClientError::InvalidMethod)?;
        Ok(MethodHandle {
            service: self.kind,
            index,
            spec,
        })
    }

    pub(crate) async fn invoke(
        &self,
        method: MethodHandle,
        payload: Vec<u8>,
        timeout_nano: u64,
    ) -> ttrpc::Result<Vec<u8>> {
        let spec = self.kind.spec();
        let request = ttrpc::Request {
            service: format!("{}.{}", spec.package, spec.service),
            method: method.spec().name.to_owned(),
            timeout_nano: timeout_nano.try_into().unwrap_or(i64::MAX),
            payload,
            ..Default::default()
        };
        self.raw
            .request(request)
            .await
            .map(|response| response.payload)
    }
}

impl fmt::Debug for ServiceHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServiceHandle")
            .field("service", &self.kind)
            .finish()
    }
}
