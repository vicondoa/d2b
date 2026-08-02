//! Installation-wide observability Provider configuration.

use serde::{Deserialize, Serialize};

/// Provider configuration error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// An unknown field or unsupported value was supplied.
    Invalid,
}

impl core::fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("observability-provider-config-invalid")
    }
}

impl std::error::Error for ConfigError {}

/// The only installation-wide setting accepted by the Provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderConfig {
    /// Whether bounded self-metrics are exposed.
    pub self_metrics_enable: bool,
}

impl Serialize for ProviderConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_json().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ProviderConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        Self::from_json(&value).map_err(|_| serde::de::Error::custom(ConfigError::Invalid))
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            self_metrics_enable: true,
        }
    }
}

impl ProviderConfig {
    /// Parse the strict root config shape.
    pub fn from_json(value: &serde_json::Value) -> Result<Self, ConfigError> {
        let object = value.as_object().ok_or(ConfigError::Invalid)?;
        let allowed = ["selfMetrics"];
        if object.keys().any(|key| !allowed.contains(&key.as_str())) {
            return Err(ConfigError::Invalid);
        }
        let self_metrics_enable = match object.get("selfMetrics") {
            None => true,
            Some(value) => {
                let object = value.as_object().ok_or(ConfigError::Invalid)?;
                if object.keys().any(|key| key != "enable") {
                    return Err(ConfigError::Invalid);
                }
                object
                    .get("enable")
                    .and_then(serde_json::Value::as_bool)
                    .ok_or(ConfigError::Invalid)?
            }
        };
        Ok(Self {
            self_metrics_enable,
        })
    }

    /// Return the canonical provider-neutral JSON shape.
    pub fn to_json(self) -> serde_json::Value {
        serde_json::json!({
            "selfMetrics": {
                "enable": self.self_metrics_enable
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_self_metrics_is_accepted() {
        let config = ProviderConfig::from_json(&serde_json::json!({})).unwrap();
        assert!(config.self_metrics_enable);
        assert!(
            ProviderConfig::from_json(&serde_json::json!({
                "serviceRef": "TelemetryService/one"
            }))
            .is_err()
        );
        assert!(
            ProviderConfig::from_json(&serde_json::json!({
                "selfMetrics": {"enable": "yes"}
            }))
            .is_err()
        );
    }
}
