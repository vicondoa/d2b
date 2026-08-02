//! In-process metric inventory used by bounded emitters.

use std::collections::BTreeMap;

use crate::metric_label_policy::{
    IdentityCanaries, LabelDescriptor, MetricDescriptor, MetricPolicyError, validate_data_point,
};

/// Metric instrument kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricKind {
    /// Monotonically increasing count.
    Counter,
    /// Current value.
    Gauge,
    /// Distribution with fixed boundaries.
    Histogram,
}

/// A value recorded for one data point.
#[derive(Debug, Clone, PartialEq)]
pub enum MetricValue {
    /// Increment or set an integer value.
    Integer(u64),
    /// Observe a duration or other scalar.
    Scalar(f64),
}

/// One metric family.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricFamily {
    descriptor: MetricDescriptor,
    kind: MetricKind,
    buckets: Vec<f64>,
    values: BTreeMap<Vec<(String, String)>, MetricValue>,
}

impl MetricFamily {
    /// Construct and validate a metric family.
    pub fn new(
        descriptor: MetricDescriptor,
        kind: MetricKind,
        buckets: impl IntoIterator<Item = f64>,
    ) -> Result<Self, MetricPolicyError> {
        crate::metric_label_policy::validate_descriptor(&descriptor)?;
        let buckets = buckets.into_iter().collect::<Vec<_>>();
        if kind != MetricKind::Histogram && !buckets.is_empty() {
            return Err(MetricPolicyError::DescriptorMalformed);
        }
        if kind == MetricKind::Histogram
            && (buckets.iter().any(|bucket| *bucket <= 0.0)
                || buckets.windows(2).any(|window| window[0] >= window[1]))
        {
            return Err(MetricPolicyError::DescriptorMalformed);
        }
        Ok(Self {
            descriptor,
            kind,
            buckets,
            values: BTreeMap::new(),
        })
    }

    /// Borrow the descriptor.
    pub fn descriptor(&self) -> &MetricDescriptor {
        &self.descriptor
    }

    /// Return the instrument kind.
    pub const fn kind(&self) -> MetricKind {
        self.kind
    }

    /// Borrow the bucket boundaries.
    pub fn buckets(&self) -> &[f64] {
        &self.buckets
    }

    /// Record a value after policy validation.
    pub fn record(
        &mut self,
        labels: &BTreeMap<String, String>,
        value: MetricValue,
        canaries: &IdentityCanaries,
    ) -> Result<(), MetricPolicyError> {
        validate_data_point(&self.descriptor, labels, canaries)?;
        if matches!(
            (&self.kind, &value),
            (
                MetricKind::Counter | MetricKind::Gauge,
                MetricValue::Scalar(_)
            ) | (MetricKind::Histogram, MetricValue::Integer(_))
        ) {
            return Err(MetricPolicyError::DescriptorMalformed);
        }
        let key = labels
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        self.values.insert(key, value);
        Ok(())
    }

    /// Return the number of recorded data points.
    pub fn data_point_count(&self) -> usize {
        self.values.len()
    }
}

/// A fixed metric registry.
#[derive(Debug, Default)]
pub struct MeterRegistry {
    families: BTreeMap<String, MetricFamily>,
}

impl MeterRegistry {
    /// Register one family.
    pub fn register(&mut self, family: MetricFamily) -> Result<(), MetricPolicyError> {
        let name = family.descriptor().name().to_owned();
        if self.families.contains_key(&name) {
            return Err(MetricPolicyError::DescriptorMalformed);
        }
        self.families.insert(name, family);
        Ok(())
    }

    /// Record a value in a registered family.
    pub fn record(
        &mut self,
        name: &str,
        labels: &BTreeMap<String, String>,
        value: MetricValue,
        canaries: &IdentityCanaries,
    ) -> Result<(), MetricPolicyError> {
        self.families
            .get_mut(name)
            .ok_or(MetricPolicyError::DescriptorMalformed)?
            .record(labels, value, canaries)
    }

    /// Borrow one family.
    pub fn family(&self, name: &str) -> Option<&MetricFamily> {
        self.families.get(name)
    }

    /// Iterate the inventory in deterministic order.
    pub fn families(&self) -> impl Iterator<Item = &MetricFamily> {
        self.families.values()
    }
}

/// Buckets for the controller commit-to-handler target.
pub const CONTROLLER_HINT_BUCKETS_SECONDS: &[f64] =
    &[0.001, 0.002, 0.005, 0.010, 0.015, 0.020, 0.030, 0.050];
/// Buckets for the process commit-to-launch target.
pub const PROCESS_LAUNCH_BUCKETS_SECONDS: &[f64] = &[
    0.001, 0.005, 0.010, 0.015, 0.020, 0.030, 0.050, 0.1, 0.5, 2.0,
];
/// Buckets for store writes.
pub const STORE_WRITE_BUCKETS_SECONDS: &[f64] = &[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.5, 1.0];

/// Build a descriptor label with a static domain.
pub fn label(key: impl Into<String>, values: &[&str]) -> LabelDescriptor {
    LabelDescriptor::new(key, values.iter().copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_buckets_are_present() {
        assert!(CONTROLLER_HINT_BUCKETS_SECONDS.contains(&0.005));
        assert!(PROCESS_LAUNCH_BUCKETS_SECONDS.contains(&0.020));
        assert!(STORE_WRITE_BUCKETS_SECONDS.contains(&0.010));
    }

    #[test]
    fn registry_rejects_identity_labels() {
        let family = MetricFamily::new(
            MetricDescriptor::new("d2b_test_total", [label("vm", &["one"])]),
            MetricKind::Counter,
            [],
        );
        assert!(family.is_err());
    }
}
