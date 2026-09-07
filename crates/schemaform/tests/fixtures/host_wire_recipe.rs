use serde::de::DeserializeOwned;
use serde_json::{Number, Value};

/// Decodes a wire message the way stock `serde_json` would have.
pub fn decode_wire<T: DeserializeOwned>(text: &str) -> serde_json::Result<T> {
    let mut value: Value = serde_json::from_str(text)?;
    canonicalize_numbers(&mut value);
    serde_json::from_value(value)
}

/// Re-spells every number that is not a machine integer as the `f64` it rounds to.
fn canonicalize_numbers(value: &mut Value) {
    match value {
        Value::Number(number) if !number.is_u64() && !number.is_i64() => {
            if let Some(float) = number.as_f64().and_then(Number::from_f64) {
                *number = float;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(canonicalize_numbers),
        Value::Object(members) => members.values_mut().for_each(canonicalize_numbers),
        _ => {}
    }
}
