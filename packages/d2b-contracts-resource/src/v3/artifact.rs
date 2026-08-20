use serde::{Deserialize, Deserializer, Serialize};

use super::execution_policy::{BoundedToken, string_schema};

pub const MAX_ARTIFACT_ID_BYTES: usize = 63;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactIdError {
    Invalid,
}

impl core::fmt::Display for ArtifactIdError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("artifact identifier is invalid")
    }
}

impl std::error::Error for ArtifactIdError {}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ArtifactId(BoundedToken);

impl ArtifactId {
    pub fn parse(value: impl Into<String>) -> Result<Self, ArtifactIdError> {
        let value = value.into();
        if value.len() > MAX_ARTIFACT_ID_BYTES {
            return Err(ArtifactIdError::Invalid);
        }
        BoundedToken::parse(value)
            .map(Self)
            .map_err(|_| ArtifactIdError::Invalid)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl core::fmt::Debug for ArtifactId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ArtifactId(<redacted>)")
    }
}

string_schema!(ArtifactId, 1, MAX_ARTIFACT_ID_BYTES);

impl<'de> Deserialize<'de> for ArtifactId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
