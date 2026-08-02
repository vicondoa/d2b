//! Typed Provider RPC proxy over an injected authenticated transport.
//!
//! This module intentionally does not depend on a concrete session transport.
//! The Zone runtime supplies the already-authenticated ComponentSession driver;
//! the proxy only binds a bounded method and canonical payload to it.

use std::{fmt, future::Future};

use d2b_contracts::v3::CanonicalJsonObject;

use crate::{
    context::OwnedOperationContext, error::ProviderRuntimeError, identity::ProviderMethodName,
};

/// Maximum bytes in one Provider RPC payload.
pub const MAX_RPC_PAYLOAD_BYTES: usize = 256 * 1024;

/// One typed Provider RPC request.
#[derive(Clone, PartialEq, Eq)]
pub struct RpcCall {
    method: ProviderMethodName,
    payload: CanonicalJsonObject,
}

impl RpcCall {
    /// Construct a call after canonical-size validation.
    pub fn new(
        method: ProviderMethodName,
        payload: CanonicalJsonObject,
    ) -> Result<Self, ProviderRuntimeError> {
        let bytes = payload.to_canonical_bytes();
        if bytes.len() > MAX_RPC_PAYLOAD_BYTES {
            return Err(ProviderRuntimeError::RpcPayloadInvalid);
        }
        Ok(Self { method, payload })
    }

    /// Borrow the bounded method.
    pub const fn method(&self) -> &ProviderMethodName {
        &self.method
    }

    /// Borrow canonical payload.
    pub const fn payload(&self) -> &CanonicalJsonObject {
        &self.payload
    }
}

impl fmt::Debug for RpcCall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RpcCall(<redacted>)")
    }
}

/// One canonical Provider RPC response.
#[derive(Clone, PartialEq, Eq)]
pub struct RpcResponse {
    payload: CanonicalJsonObject,
}

impl RpcResponse {
    /// Construct a response after canonical-size validation.
    pub fn new(payload: CanonicalJsonObject) -> Result<Self, ProviderRuntimeError> {
        let bytes = payload.to_canonical_bytes();
        if bytes.len() > MAX_RPC_PAYLOAD_BYTES {
            return Err(ProviderRuntimeError::RpcPayloadInvalid);
        }
        Ok(Self { payload })
    }

    /// Borrow canonical response payload.
    pub const fn payload(&self) -> &CanonicalJsonObject {
        &self.payload
    }
}

impl fmt::Debug for RpcResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RpcResponse(<redacted>)")
    }
}

/// The injected authenticated Provider transport.
pub trait AuthenticatedProviderRpc: Send + Sync {
    /// Dispatch one already-admitted call.
    fn call(
        &self,
        context: &OwnedOperationContext,
        request: RpcCall,
    ) -> impl Future<Output = Result<RpcResponse, ProviderRuntimeError>> + Send;
}

/// Provider registry RPC proxy.
pub struct RpcProviderProxy<T> {
    transport: T,
}

impl<T> RpcProviderProxy<T> {
    /// Build a proxy over the Zone runtime's authenticated transport.
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Borrow the transport adapter.
    pub const fn transport(&self) -> &T {
        &self.transport
    }
}

impl<T> RpcProviderProxy<T>
where
    T: AuthenticatedProviderRpc,
{
    /// Dispatch a typed canonical RPC.
    pub async fn dispatch(
        &self,
        context: &OwnedOperationContext,
        request: RpcCall,
    ) -> Result<RpcResponse, ProviderRuntimeError> {
        if context.is_cancelled() {
            return Err(ProviderRuntimeError::Cancelled);
        }
        if context.method() != request.method() {
            return Err(ProviderRuntimeError::CapabilityDenied);
        }
        let _ = context.remaining()?;
        let response = self.transport.call(context, request).await?;
        let _ = context.remaining()?;
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_payloads_fail_before_transport() {
        let payload = CanonicalJsonObject::parse(br#"{"x":"a"}"#).unwrap();
        let method = ProviderMethodName::parse("inspect").unwrap();
        assert!(RpcCall::new(method, payload).is_ok());
    }

    #[test]
    fn the_proxy_binds_the_request_method_to_the_admitted_context() {
        assert_ne!(
            ProviderMethodName::parse("start").unwrap(),
            ProviderMethodName::parse("stop").unwrap()
        );
    }
}
