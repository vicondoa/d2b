//! Payload JSON Schemas for committed command and operation rows.
//!
//! A `Command` row describes the parameters one launch shape accepts, and the
//! spawn `Operation` materialized from that command reuses the same document
//! as the envelope's payload contract. Both resolve to this one type, so the
//! declared shape and the validated shape cannot drift.
//!
//! A payload schema is a closed object: `additionalProperties` is always
//! `false`, so an unknown parameter is a refusal rather than an ignored
//! field. A property marked `writeOnly` names secret material, so it never
//! carries a `default`, `enum`, `const`, or `examples` value: a secret that
//! can be inferred from the schema is not a secret.

use schemars::{JsonSchema, r#gen::SchemaGenerator, schema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Maximum serialized bytes of one payload schema document.
pub const MAX_PAYLOAD_SCHEMA_BYTES: usize = 64 * 1024;
/// Maximum declared properties of one payload object.
pub const MAX_PAYLOAD_PROPERTIES: usize = 64;
/// Maximum nesting depth of object-typed payload properties.
pub const MAX_PAYLOAD_SCHEMA_DEPTH: usize = 8;
/// Maximum bytes of one payload property name.
pub const MAX_PAYLOAD_PROPERTY_NAME_BYTES: usize = 63;
/// Property keys a `writeOnly` property must never carry.
const WRITE_ONLY_VALUE_KEYS: [&str; 4] = ["default", "enum", "const", "examples"];

/// A validated payload JSON Schema document.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PayloadSchema(Value);

impl PayloadSchema {
    /// Validate one authored payload schema.
    pub fn parse(value: Value) -> Result<Self, PayloadSchemaError> {
        let encoded = serde_json::to_vec(&value).map_err(|_| PayloadSchemaError::NotAnObject)?;
        if encoded.len() > MAX_PAYLOAD_SCHEMA_BYTES {
            return Err(PayloadSchemaError::TooLarge);
        }
        validate_object_schema(&value, 0)?;
        Ok(Self(value))
    }

    /// The schema document.
    pub fn as_value(&self) -> &Value {
        &self.0
    }

    /// The declared property names, in authored order.
    pub fn property_names(&self) -> impl Iterator<Item = &str> {
        self.properties().keys().map(String::as_str)
    }

    /// The declared property schema for one name.
    pub fn property(&self, name: &str) -> Option<&Value> {
        self.properties().get(name)
    }

    /// Whether the schema declares a property of this name.
    pub fn declares(&self, name: &str) -> bool {
        self.properties().contains_key(name)
    }

    /// Whether one declared property carries secret material.
    pub fn is_write_only(&self, name: &str) -> bool {
        self.property(name)
            .and_then(Value::as_object)
            .and_then(|property| property.get("writeOnly"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// The required property names.
    pub fn required_names(&self) -> impl Iterator<Item = &str> {
        self.0
            .as_object()
            .and_then(|object| object.get("required"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_str)
    }

    fn properties(&self) -> &Map<String, Value> {
        self.0
            .as_object()
            .and_then(|object| object.get("properties"))
            .and_then(Value::as_object)
            .expect("validated payload schemas always carry a properties object")
    }
}

impl core::fmt::Debug for PayloadSchema {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Payload schemas may describe secret-bearing shapes; diagnostics
        // report the property set, never the document.
        formatter
            .debug_struct("PayloadSchema")
            .field("properties", &self.property_names().collect::<Vec<_>>())
            .finish()
    }
}

impl JsonSchema for PayloadSchema {
    fn schema_name() -> String {
        "PayloadSchema".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> schema::Schema {
        let mut schema = schema::SchemaObject {
            instance_type: Some(schema::SingleOrVec::Single(Box::new(
                schema::InstanceType::Object,
            ))),
            ..Default::default()
        };
        schema.metadata().description = Some(
            "Closed payload JSON Schema: object-typed, additionalProperties=false, \
             writeOnly properties carrying no default/enum/const/examples."
                .to_owned(),
        );
        schema::Schema::Object(schema)
    }
}

/// One invalid payload schema document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadSchemaError {
    /// The document is not a JSON object.
    NotAnObject,
    /// The document is not typed `object`.
    NotObjectTyped,
    /// `additionalProperties` is not `false`.
    OpenProperties,
    /// A nested object schema is over the depth bound.
    TooDeep,
    /// The document declares more than [`MAX_PAYLOAD_PROPERTIES`] properties.
    TooManyProperties,
    /// A property name is empty, over bound, or not lower-camel.
    InvalidPropertyName,
    /// A property value is not a JSON object.
    InvalidProperty,
    /// `required` is not an array of declared property names.
    InvalidRequired,
    /// A `writeOnly` property carries a `default`, `enum`, `const`, or
    /// `examples` value.
    WriteOnlyCarriesValue,
    /// The document is over the serialized byte bound.
    TooLarge,
}

impl core::fmt::Display for PayloadSchemaError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            Self::NotAnObject => "payload schema is not a JSON object",
            Self::NotObjectTyped => "payload schema is not typed object",
            Self::OpenProperties => "payload schema declares open properties",
            Self::TooDeep => "payload schema nests deeper than the object bound",
            Self::TooManyProperties => "payload schema declares too many properties",
            Self::InvalidPropertyName => "payload schema carries an invalid property name",
            Self::InvalidProperty => "payload schema carries a non-object property",
            Self::InvalidRequired => "payload schema required list is invalid",
            Self::WriteOnlyCarriesValue => {
                "payload schema gives a writeOnly property a default, enum, const, or examples value"
            }
            Self::TooLarge => "payload schema is over the byte bound",
        };
        formatter.write_str(text)
    }
}

impl std::error::Error for PayloadSchemaError {}

/// Whether one name is a well-formed payload property name.
pub fn valid_property_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let head = matches!(bytes.next(), Some(b'a'..=b'z'));
    head && name.len() <= MAX_PAYLOAD_PROPERTY_NAME_BYTES
        && bytes.all(|byte| byte.is_ascii_alphanumeric())
}

fn validate_object_schema(schema: &Value, depth: usize) -> Result<(), PayloadSchemaError> {
    let Some(object) = schema.as_object() else {
        return Err(PayloadSchemaError::NotAnObject);
    };
    if object.get("type").and_then(Value::as_str) != Some("object") {
        return Err(PayloadSchemaError::NotObjectTyped);
    }
    if object.get("additionalProperties") != Some(&Value::Bool(false)) {
        return Err(PayloadSchemaError::OpenProperties);
    }
    let Some(properties) = object.get("properties").and_then(Value::as_object) else {
        return Err(PayloadSchemaError::InvalidProperty);
    };
    if properties.len() > MAX_PAYLOAD_PROPERTIES {
        return Err(PayloadSchemaError::TooManyProperties);
    }
    match object.get("required") {
        None => {}
        Some(Value::Array(names)) => {
            for name in names {
                let Some(name) = name.as_str() else {
                    return Err(PayloadSchemaError::InvalidRequired);
                };
                if !properties.contains_key(name) {
                    return Err(PayloadSchemaError::InvalidRequired);
                }
            }
        }
        Some(_) => return Err(PayloadSchemaError::InvalidRequired),
    }
    for (name, property) in properties {
        if !valid_property_name(name) {
            return Err(PayloadSchemaError::InvalidPropertyName);
        }
        let Some(fields) = property.as_object() else {
            return Err(PayloadSchemaError::InvalidProperty);
        };
        if fields.get("writeOnly").and_then(Value::as_bool) == Some(true)
            && WRITE_ONLY_VALUE_KEYS
                .iter()
                .any(|key| fields.contains_key(*key))
        {
            return Err(PayloadSchemaError::WriteOnlyCarriesValue);
        }
        if fields.get("type").and_then(Value::as_str) == Some("object") {
            if depth + 1 >= MAX_PAYLOAD_SCHEMA_DEPTH {
                return Err(PayloadSchemaError::TooDeep);
            }
            validate_object_schema(property, depth + 1)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["socketPath"],
            "properties": {
                "socketPath": { "type": "string", "pattern": "^/run/d2b/vfs/[A-Za-z0-9._-]+$" },
                "supervisorToken": { "type": "string", "writeOnly": true, "minLength": 32 },
                "workerCount": { "type": "integer", "minimum": 1, "maximum": 64, "default": 2 }
            }
        })
    }

    #[test]
    fn a_closed_object_schema_is_admitted() {
        let schema = PayloadSchema::parse(object_schema()).expect("schema validates");
        assert_eq!(
            schema.property_names().collect::<Vec<_>>(),
            vec!["socketPath", "supervisorToken", "workerCount"]
        );
        assert_eq!(schema.required_names().collect::<Vec<_>>(), vec!["socketPath"]);
        assert!(schema.declares("supervisorToken"));
        assert!(schema.is_write_only("supervisorToken"));
        assert!(!schema.is_write_only("socketPath"));
    }

    #[test]
    fn open_or_untyped_documents_are_refused() {
        for rejected in [
            json!({ "type": "object", "properties": {} }),
            json!({ "type": "object", "additionalProperties": true, "properties": {} }),
            json!({ "type": "array", "additionalProperties": false, "properties": {} }),
            json!("not-an-object"),
        ] {
            assert!(PayloadSchema::parse(rejected).is_err());
        }
    }

    #[test]
    fn write_only_properties_reject_inferable_values() {
        for banned in ["default", "enum", "const", "examples"] {
            let value = match banned {
                "enum" => json!(["a", "b"]),
                "examples" => json!(["a"]),
                _ => json!("secret"),
            };
            let mut property = json!({ "type": "string", "writeOnly": true });
            property[banned] = value;
            let schema = json!({
                "type": "object",
                "additionalProperties": false,
                "properties": { "token": property }
            });
            assert_eq!(
                PayloadSchema::parse(schema),
                Err(PayloadSchemaError::WriteOnlyCarriesValue)
            );
        }
    }

    #[test]
    fn unknown_required_names_and_malformed_property_names_are_refused() {
        assert_eq!(
            PayloadSchema::parse(json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["missing"],
                "properties": {}
            })),
            Err(PayloadSchemaError::InvalidRequired)
        );
        for name in ["Upper", "with space", "", "1digit"] {
            let schema = json!({
                "type": "object",
                "additionalProperties": false,
                "properties": { name: { "type": "string" } }
            });
            assert_eq!(
                PayloadSchema::parse(schema),
                Err(PayloadSchemaError::InvalidPropertyName),
                "{name:?} must be refused"
            );
        }
    }

    #[test]
    fn nested_object_properties_keep_the_closed_shape() {
        let nested = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "inner": {
                    "type": "object",
                    "additionalProperties": true,
                    "properties": {}
                }
            }
        });
        assert_eq!(
            PayloadSchema::parse(nested),
            Err(PayloadSchemaError::OpenProperties)
        );
    }

    #[test]
    fn diagnostics_never_render_property_values() {
        let marker = format!("secret-{:x}", std::process::id());
        let mut schema = object_schema();
        schema["properties"]["supervisorToken"]["description"] = Value::String(marker.clone());
        let parsed = PayloadSchema::parse(schema).expect("schema validates");
        assert!(!format!("{parsed:?}").contains(&marker));
    }
}
