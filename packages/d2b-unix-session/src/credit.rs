use d2b_contracts::v2_component_session::{
    MAX_HOST_ATTACHMENT_CREDITS, MAX_PROCESS_ATTACHMENT_CREDITS, RESERVED_CONTROL_FDS,
};
use rustix::process::{Resource, getrlimit};
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreditError {
    ZeroLimit,
    BaselineExceedsLimit,
    Exhausted,
    Overflow,
}

#[derive(Clone)]
pub struct CreditPool {
    inner: Arc<CreditPoolInner>,
}

struct CreditPoolInner {
    limit: usize,
    used: AtomicUsize,
}

impl fmt::Debug for CreditPool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreditPool")
            .field("limit", &self.limit())
            .field("available", &self.available())
            .finish()
    }
}

impl CreditPool {
    pub fn new(limit: usize) -> Result<Self, CreditError> {
        if limit == 0 {
            return Err(CreditError::ZeroLimit);
        }
        Ok(Self {
            inner: Arc::new(CreditPoolInner {
                limit,
                used: AtomicUsize::new(0),
            }),
        })
    }

    pub fn limit(&self) -> usize {
        self.inner.limit
    }

    pub fn used(&self) -> usize {
        self.inner.used.load(Ordering::Acquire)
    }

    pub fn available(&self) -> usize {
        self.limit().saturating_sub(self.used())
    }

    fn reserve(&self, amount: usize) -> Result<ScopeReservation, CreditError> {
        if amount == 0 {
            return Ok(ScopeReservation {
                pool: self.clone(),
                amount: 0,
                active: false,
            });
        }
        let mut current = self.inner.used.load(Ordering::Acquire);
        loop {
            let next = current.checked_add(amount).ok_or(CreditError::Overflow)?;
            if next > self.inner.limit {
                return Err(CreditError::Exhausted);
            }
            match self.inner.used.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(ScopeReservation {
                        pool: self.clone(),
                        amount,
                        active: true,
                    });
                }
                Err(observed) => current = observed,
            }
        }
    }
}

struct ScopeReservation {
    pool: CreditPool,
    amount: usize,
    active: bool,
}

impl ScopeReservation {
    fn release(&mut self) {
        if self.active {
            let previous = self
                .pool
                .inner
                .used
                .fetch_sub(self.amount, Ordering::AcqRel);
            debug_assert!(previous >= self.amount);
            self.active = false;
        }
    }
}

impl Drop for ScopeReservation {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreditScope {
    Packet = 0,
    Request = 1,
    Operation = 2,
    Session = 3,
    Process = 4,
    Host = 5,
}

#[derive(Clone)]
pub struct CreditScopeSet {
    packet: CreditPool,
    request: CreditPool,
    operation: CreditPool,
    session: CreditPool,
    process: CreditPool,
    host: CreditPool,
}

impl CreditScopeSet {
    pub fn new(
        packet: CreditPool,
        request: CreditPool,
        operation: CreditPool,
        session: CreditPool,
        process: CreditPool,
        host: CreditPool,
    ) -> Self {
        Self {
            packet,
            request,
            operation,
            session,
            process,
            host,
        }
    }

    pub fn reserve(&self, amount: usize) -> Result<CreditBundle, CreditError> {
        let mut reservations: [Option<ScopeReservation>; 6] = Default::default();
        for (index, pool) in self.pools().into_iter().enumerate() {
            reservations[index] = Some(pool.reserve(amount)?);
        }
        Ok(CreditBundle { reservations })
    }

    pub fn reserve_ingress(&self, amount: usize) -> Result<CreditBundle, CreditError> {
        let mut reservations: [Option<ScopeReservation>; 6] = Default::default();
        for scope in [
            CreditScope::Packet,
            CreditScope::Session,
            CreditScope::Process,
            CreditScope::Host,
        ] {
            reservations[scope as usize] = Some(self.pool(scope).reserve(amount)?);
        }
        Ok(CreditBundle { reservations })
    }

    fn pools(&self) -> [&CreditPool; 6] {
        [
            &self.packet,
            &self.request,
            &self.operation,
            &self.session,
            &self.process,
            &self.host,
        ]
    }

    fn pool(&self, scope: CreditScope) -> &CreditPool {
        self.pools()[scope as usize]
    }
}

pub struct CreditBundle {
    reservations: [Option<ScopeReservation>; 6],
}

impl fmt::Debug for CreditBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreditBundle")
            .field(
                "active_scopes",
                &self
                    .reservations
                    .iter()
                    .flatten()
                    .filter(|reservation| reservation.active)
                    .count(),
            )
            .finish()
    }
}

impl CreditBundle {
    pub fn release(&mut self, scope: CreditScope) {
        if let Some(reservation) = &mut self.reservations[scope as usize] {
            reservation.release();
        }
    }

    pub fn acquire_dispatch(
        &mut self,
        scopes: &CreditScopeSet,
        amount: usize,
    ) -> Result<(), CreditError> {
        if self.reservations[CreditScope::Request as usize].is_some()
            || self.reservations[CreditScope::Operation as usize].is_some()
        {
            return Err(CreditError::Exhausted);
        }
        let request = scopes.pool(CreditScope::Request).reserve(amount)?;
        let operation = scopes.pool(CreditScope::Operation).reserve(amount)?;
        self.reservations[CreditScope::Request as usize] = Some(request);
        self.reservations[CreditScope::Operation as usize] = Some(operation);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessCreditLimit {
    transferable: usize,
}

impl ProcessCreditLimit {
    pub fn from_current(observed_nontransferable_open_fds: usize) -> Result<Self, CreditError> {
        let soft = getrlimit(Resource::Nofile).current.unwrap_or(u64::MAX);
        Self::derive(soft, observed_nontransferable_open_fds)
    }

    pub fn derive(
        rlimit_nofile_soft: u64,
        observed_nontransferable_open_fds: usize,
    ) -> Result<Self, CreditError> {
        let soft = usize::try_from(rlimit_nofile_soft).map_err(|_| CreditError::Overflow)?;
        let baseline_and_reserve = observed_nontransferable_open_fds
            .checked_add(usize::from(RESERVED_CONTROL_FDS))
            .ok_or(CreditError::Overflow)?;
        if baseline_and_reserve >= soft {
            return Err(CreditError::BaselineExceedsLimit);
        }
        Ok(Self {
            transferable: (soft - baseline_and_reserve)
                .min(usize::from(MAX_PROCESS_ATTACHMENT_CREDITS)),
        })
    }

    pub fn transferable(self) -> usize {
        self.transferable
    }

    pub fn process_pool(self) -> Result<CreditPool, CreditError> {
        CreditPool::new(self.transferable)
    }

    pub fn host_pool(configured_limit: usize) -> Result<CreditPool, CreditError> {
        CreditPool::new(configured_limit.min(usize::from(MAX_HOST_ATTACHMENT_CREDITS)))
    }
}
