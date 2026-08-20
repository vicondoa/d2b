use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize};

pub const MAX_CONTRACT_TOKEN_LEN: usize = 160;
pub const MAX_PATH_TEMPLATE_LEN: usize = 1024;
pub const MAX_CONTRACT_TEXT_LEN: usize = 512;

const TOKEN_PATTERN: &str = "^[A-Za-z0-9][A-Za-z0-9._:/@+-]*$";
const SLUG_PATTERN: &str = "^[a-z][a-z0-9-]*$";
const PATH_PATTERN: &str = "^/[^\\u0000]*$";
const TEXT_PATTERN: &str = "^[^\\u0000]*$";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractStringError {
    Empty,
    TooLong { max: usize },
    BadShape,
}

impl std::fmt::Display for ContractStringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("value is empty"),
            Self::TooLong { max } => write!(f, "value exceeds {max} bytes"),
            Self::BadShape => f.write_str("value has an invalid shape"),
        }
    }
}

impl std::error::Error for ContractStringError {}

fn bounded(raw: String, max: usize) -> Result<String, ContractStringError> {
    if raw.is_empty() {
        return Err(ContractStringError::Empty);
    }
    if raw.len() > max {
        return Err(ContractStringError::TooLong { max });
    }
    if raw.contains('\0') {
        return Err(ContractStringError::BadShape);
    }
    Ok(raw)
}

fn token(raw: String) -> Result<String, ContractStringError> {
    let raw = bounded(raw, MAX_CONTRACT_TOKEN_LEN)?;
    if raw
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '@' | '+' | '-'))
        && raw
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        Ok(raw)
    } else {
        Err(ContractStringError::BadShape)
    }
}

fn slug(raw: String) -> Result<String, ContractStringError> {
    let raw = bounded(raw, MAX_CONTRACT_TOKEN_LEN)?;
    let mut chars = raw.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return Err(ContractStringError::BadShape),
    }
    if chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        Ok(raw)
    } else {
        Err(ContractStringError::BadShape)
    }
}

fn path_template(raw: String) -> Result<String, ContractStringError> {
    let raw = bounded(raw, MAX_PATH_TEMPLATE_LEN)?;
    if raw.starts_with('/') {
        Ok(raw)
    } else {
        Err(ContractStringError::BadShape)
    }
}

fn text(raw: String) -> Result<String, ContractStringError> {
    bounded(raw, MAX_CONTRACT_TEXT_LEN)
}

macro_rules! contract_string {
    ($name:ident, $parse_fn:ident, $max:expr, $pattern:expr, $description:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(raw: impl Into<String>) -> Result<Self, ContractStringError> {
                $parse_fn(raw.into()).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> String {
                stringify!($name).to_owned()
            }

            fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
                Schema::Object(SchemaObject {
                    instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
                    string: Some(Box::new(StringValidation {
                        max_length: Some($max as u32),
                        min_length: Some(1),
                        pattern: Some($pattern.to_owned()),
                    })),
                    metadata: Some(Box::new(schemars::schema::Metadata {
                        description: Some($description.to_owned()),
                        ..Default::default()
                    })),
                    ..Default::default()
                })
            }
        }
    };
}

contract_string!(
    ContractId,
    token,
    MAX_CONTRACT_TOKEN_LEN,
    TOKEN_PATTERN,
    "Bounded storage/synchronization contract identifier."
);
contract_string!(
    ReasonSlug,
    slug,
    MAX_CONTRACT_TOKEN_LEN,
    SLUG_PATTERN,
    "Closed degraded-state or audit reason slug."
);
contract_string!(
    PathTemplate,
    path_template,
    MAX_PATH_TEMPLATE_LEN,
    PATH_PATTERN,
    "Absolute path template with typed variables expanded by the broker."
);
contract_string!(
    ContractText,
    text,
    MAX_CONTRACT_TEXT_LEN,
    TEXT_PATTERN,
    "Bounded human-readable contract text."
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_bounded_and_shaped() {
        assert!(ContractId::parse("storage:path/etc").is_ok());
        assert!(ContractId::parse("").is_err());
        assert!(ContractId::parse("bad space").is_err());
        assert!(ContractId::parse(format!("a{}", "b".repeat(MAX_CONTRACT_TOKEN_LEN))).is_err());
    }

    #[test]
    fn slugs_are_lowercase() {
        assert!(ReasonSlug::parse("storage-drift").is_ok());
        assert!(ReasonSlug::parse("StorageDrift").is_err());
    }

    #[test]
    fn path_templates_are_absolute() {
        assert!(PathTemplate::parse("/run/d2b/<vm>").is_ok());
        assert!(PathTemplate::parse("relative/path").is_err());
        assert!(PathTemplate::parse("/bad\0path").is_err());
    }
}
