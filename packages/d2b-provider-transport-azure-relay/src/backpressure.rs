//! Credit-bounded Relay stream buffering.

/// Maximum frame accepted by the Provider.
pub const MAX_RELAY_FRAME_BYTES: usize = 64 * 1024;

/// Backpressure failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackpressureError {
    /// A frame exceeds the fixed frame bound.
    FrameTooLarge,
    /// Aggregate credits are exhausted.
    CreditExhausted,
}

/// Bounded credit window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreditWindow {
    max_bytes: usize,
    available: usize,
    in_flight: usize,
}

impl CreditWindow {
    /// Construct a nonzero bounded window.
    pub fn new(max_bytes: usize) -> Result<Self, BackpressureError> {
        if max_bytes == 0 || max_bytes > 16 * 1024 * 1024 {
            return Err(BackpressureError::CreditExhausted);
        }
        Ok(Self {
            max_bytes,
            available: max_bytes,
            in_flight: 0,
        })
    }

    /// Reserve credits for one frame.
    pub fn reserve(&mut self, bytes: usize) -> Result<(), BackpressureError> {
        if bytes > MAX_RELAY_FRAME_BYTES {
            return Err(BackpressureError::FrameTooLarge);
        }
        if bytes > self.available {
            return Err(BackpressureError::CreditExhausted);
        }
        self.available -= bytes;
        self.in_flight += bytes;
        Ok(())
    }

    /// Return credits after a frame is written.
    pub fn complete(&mut self, bytes: usize) {
        let returned = bytes.min(self.in_flight);
        self.in_flight -= returned;
        self.available = (self.available + returned).min(self.max_bytes);
    }

    /// Grant additional credits from the remote named stream.
    pub fn grant(&mut self, bytes: usize) {
        self.available = (self.available + bytes).min(self.max_bytes);
    }

    /// Return current available credits.
    pub const fn available(&self) -> usize {
        self.available
    }

    /// Return bytes in flight.
    pub const fn in_flight(&self) -> usize {
        self.in_flight
    }
}
