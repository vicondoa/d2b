//! Opaque display proxy principal allocation.

use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Principal pool construction or allocation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrincipalPoolError {
    /// A configured session name was empty or duplicated.
    InvalidSession,
    /// The dynamic pool size was outside the signed bound.
    InvalidPoolSize,
    /// All pre-provisioned pool accounts are currently occupied.
    NoPrincipalAvailable,
    /// A lease did not belong to this pool.
    UnknownLease,
}

impl core::fmt::Display for PrincipalPoolError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSession => "display-principal-session-invalid",
            Self::InvalidPoolSize => "display-principal-pool-invalid",
            Self::NoPrincipalAvailable => "no-principal-available",
            Self::UnknownLease => "display-principal-lease-unknown",
        })
    }
}

impl std::error::Error for PrincipalPoolError {}

/// Opaque lease for one pre-provisioned display principal.
#[derive(PartialEq, Eq)]
pub struct PrincipalLease {
    index: usize,
    principal: String,
}

impl PrincipalLease {
    /// Return the opaque account name selected for the worker.
    pub fn principal(&self) -> &str {
        &self.principal
    }
}

impl core::fmt::Debug for PrincipalLease {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("PrincipalLease(REDACTED)")
    }
}

/// Bounded pool of pre-provisioned proxy principals.
pub struct PrincipalPool {
    bundle: BTreeMap<String, String>,
    dynamic: Vec<String>,
    occupied: BTreeSet<usize>,
}

impl PrincipalPool {
    /// Build a pool for bundle-declared sessions and dynamic sessions.
    pub fn new<I, S>(bundle_sessions: I, pool_size: usize) -> Result<Self, PrincipalPoolError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        if pool_size == 0 || pool_size > 32 {
            return Err(PrincipalPoolError::InvalidPoolSize);
        }
        let mut bundle = BTreeMap::new();
        for session in bundle_sessions {
            let session = session.into();
            if session.is_empty()
                || bundle
                    .insert(session.clone(), Self::principal_for("", &session))
                    .is_some()
            {
                return Err(PrincipalPoolError::InvalidSession);
            }
        }
        let dynamic = (0..pool_size).map(Self::pool_principal).collect();
        Ok(Self {
            bundle,
            dynamic,
            occupied: BTreeSet::new(),
        })
    }

    /// Derive an opaque hash-based principal for a bundle session.
    pub fn principal_for(zone: &str, session: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"d2b-wlp:");
        hasher.update(zone.as_bytes());
        hasher.update(b":");
        hasher.update(session.as_bytes());
        let digest = hasher.finalize();
        let mut short = String::with_capacity(24);
        for byte in &digest[..6] {
            use core::fmt::Write as _;
            let _ = write!(short, "{byte:02x}");
        }
        format!("d2b-wlp-{short}")
    }

    /// Derive an opaque dynamic pool account name.
    pub fn pool_principal(index: usize) -> String {
        format!("d2b-wlp-p{index}")
    }

    /// Return the bundle principal, if a session was declared.
    pub fn bundle_principal(&self, session: &str) -> Option<&str> {
        self.bundle.get(session).map(String::as_str)
    }

    /// Acquire one dynamic pool account.
    pub fn acquire_dynamic(&mut self) -> Result<PrincipalLease, PrincipalPoolError> {
        let index = (0..self.dynamic.len())
            .find(|index| !self.occupied.contains(index))
            .ok_or(PrincipalPoolError::NoPrincipalAvailable)?;
        self.occupied.insert(index);
        Ok(PrincipalLease {
            index,
            principal: self.dynamic[index].clone(),
        })
    }

    /// Return whether a lease is currently owned by this pool.
    pub fn owns(&self, lease: &PrincipalLease) -> bool {
        lease.index < self.dynamic.len()
            && self.dynamic[lease.index] == lease.principal
            && self.occupied.contains(&lease.index)
    }

    /// Release a dynamic pool account.
    pub fn release(&mut self, lease: PrincipalLease) -> Result<(), PrincipalPoolError> {
        if lease.index >= self.dynamic.len()
            || self.dynamic[lease.index] != lease.principal
            || !self.occupied.remove(&lease.index)
        {
            return Err(PrincipalPoolError::UnknownLease);
        }
        Ok(())
    }

    /// Return the number of available dynamic accounts.
    pub fn available(&self) -> usize {
        self.dynamic.len().saturating_sub(self.occupied.len())
    }

    /// Return the total number of provisioned accounts.
    pub fn provisioned(&self) -> usize {
        self.bundle.len() + self.dynamic.len()
    }
}

impl core::fmt::Debug for PrincipalPool {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PrincipalPool")
            .field("bundle_count", &self.bundle.len())
            .field("dynamic_count", &self.dynamic.len())
            .field("occupied_count", &self.occupied.len())
            .finish()
    }
}
