//! Canonical root configuration schema emission.

use d2b_contracts::v3::canonical_json_bytes;

/// The root JSON Schema written to a Provider artifact.
///
/// This alias keeps the toolkit's public name tied to the `schemars` root
/// schema representation used by the workspace.
pub type RootConfigSchema = schemars::schema::RootSchema;

/// Emit a root configuration schema as exact `d2b-cjson/v1` bytes.
///
/// The returned buffer has no pretty-printing, BOM, or trailing newline.
/// Schema construction is responsible for supplying values accepted by the
/// canonical JSON profile; failure to serialize such a root schema is a
/// programming error.
pub fn emit_canonical(schema: &RootConfigSchema) -> Vec<u8> {
    let mut value =
        serde_json::to_value(schema).expect("RootConfigSchema must be serializable as JSON");
    normalize_integral_numbers(&mut value);
    canonical_json_bytes(&value).expect("RootConfigSchema must be canonicalizable")
}

fn normalize_integral_numbers(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                normalize_integral_numbers(value);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values_mut() {
                normalize_integral_numbers(value);
            }
        }
        serde_json::Value::Number(number) if number.as_i64().is_none() => {
            let Some(float) = number.as_f64() else {
                return;
            };
            if float.is_finite() && float.fract() == 0.0 {
                let integer = i64::try_from(float as i128)
                    .expect("integral schema number must fit a signed 64-bit integer");
                *number = serde_json::Number::from(integer);
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;

    #[derive(JsonSchema)]
    struct Config {
        #[allow(dead_code)]
        count: u32,
    }

    #[test]
    fn generated_schema_is_canonicalizable() {
        let schema = schemars::schema_for!(Config);
        let bytes = emit_canonical(&schema);
        assert_ne!(bytes.last(), Some(&b'\n'));
    }
}
