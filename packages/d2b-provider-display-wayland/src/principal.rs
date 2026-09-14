//! Opaque display proxy principal allocation.

use std::collections::BTreeSet;

/// Principal pool construction or allocation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrincipalPoolError {
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
    dynamic: Vec<String>,
    occupied: BTreeSet<usize>,
}

impl PrincipalPool {
    /// Build a pool of dynamic pre-provisioned proxy principals.
    pub fn new(pool_size: usize) -> Result<Self, PrincipalPoolError> {
        if pool_size == 0 || pool_size > 32 {
            return Err(PrincipalPoolError::InvalidPoolSize);
        }
        let dynamic = (0..pool_size).map(Self::pool_principal).collect();
        Ok(Self {
            dynamic,
            occupied: BTreeSet::new(),
        })
    }

    /// Derive an opaque dynamic pool account name.
    pub fn pool_principal(index: usize) -> String {
        format!("d2b-wlp-p{index}")
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
        self.dynamic.len()
    }
}

impl core::fmt::Debug for PrincipalPool {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PrincipalPool")
            .field("dynamic_count", &self.dynamic.len())
            .field("occupied_count", &self.occupied.len())
            .finish()
    }
}
