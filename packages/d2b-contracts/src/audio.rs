use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Metadata, Schema, SchemaObject, SingleOrVec},
};
use serde::{Deserialize, Deserializer, Serialize};

/// A volume or gain level in the range `0..=100`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct LevelPercent(u8);

impl LevelPercent {
    pub fn new(value: u8) -> Result<Self, LevelPercentError> {
        if value > 100 {
            return Err(LevelPercentError::OutOfRange(value));
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelPercentError {
    OutOfRange(u8),
}

impl std::fmt::Display for LevelPercentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfRange(value) => write!(f, "level {value} is out of range; must be 0..=100"),
        }
    }
}

impl std::error::Error for LevelPercentError {}

impl<'de> Deserialize<'de> for LevelPercent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u8::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for LevelPercent {
    fn schema_name() -> String {
        "LevelPercent".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Integer))),
            number: Some(Box::new(schemars::schema::NumberValidation {
                minimum: Some(0.0),
                maximum: Some(100.0),
                ..Default::default()
            })),
            metadata: Some(Box::new(Metadata {
                description: Some("Audio level in the range 0..=100.".to_owned()),
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
    }
}
