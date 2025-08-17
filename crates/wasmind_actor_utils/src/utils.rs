use serde::{Deserialize, Deserializer};

/// Generate a random alphanumeric ID of the specified length
pub fn generate_id(len: usize) -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::rng();
    (0..len)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

/// Deserializer that accepts both boolean values and string representations of booleans.
///
/// This is useful when dealing with LLM-generated JSON where booleans might be
/// represented as strings like "true" or "True" instead of actual boolean values.
///
/// # Examples
///
/// ```
/// use serde::Deserialize;
/// use wasmind_actor_utils::utils::deserialize_flexible_bool;
///
/// #[derive(Deserialize)]
/// struct MyStruct {
///     #[serde(default, deserialize_with = "deserialize_flexible_bool")]
///     my_bool: Option<bool>,
/// }
///
/// // Works with actual boolean
/// let json = r#"{"my_bool": true}"#;
/// let result: MyStruct = serde_json::from_str(json).unwrap();
/// assert_eq!(result.my_bool, Some(true));
///
/// // Works with string boolean
/// let json = r#"{"my_bool": "false"}"#;
/// let result: MyStruct = serde_json::from_str(json).unwrap();
/// assert_eq!(result.my_bool, Some(false));
///
/// // Works with mixed case
/// let json = r#"{"my_bool": "True"}"#;
/// let result: MyStruct = serde_json::from_str(json).unwrap();
/// assert_eq!(result.my_bool, Some(true));
/// ```
///
/// Accepts:
/// - Boolean values: `true`, `false`
/// - String values: `"true"`, `"false"`, `"True"`, `"False"`, `"TRUE"`, `"FALSE"`,
///   `"yes"`, `"no"`, `"Yes"`, `"No"`, `"YES"`, `"NO"`,
///   `"1"`, `"0"`
pub fn deserialize_flexible_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum FlexibleBool {
        Bool(bool),
        String(String),
        Null,
    }

    match FlexibleBool::deserialize(deserializer)? {
        FlexibleBool::Bool(b) => Ok(Some(b)),
        FlexibleBool::String(s) => match s.to_lowercase().as_str() {
            "true" | "yes" | "1" => Ok(Some(true)),
            "false" | "no" | "0" => Ok(Some(false)),
            _ => Err(serde::de::Error::custom(format!(
                "Invalid boolean string '{s}'. Expected: true, false, yes, no, 1, or 0 (case-insensitive)"
            ))),
        },
        FlexibleBool::Null => Ok(None),
    }
}

/// Non-optional version of `deserialize_flexible_bool`.
///
/// Use this when the boolean field is required (not wrapped in Option).
///
/// # Examples
///
/// ```ignore
/// #[derive(serde::Deserialize)]
/// struct MyStruct {
///     #[serde(deserialize_with = "deserialize_flexible_bool_required")]
///     my_bool: bool,
/// }
/// ```
pub fn deserialize_flexible_bool_required<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_flexible_bool(deserializer)?
        .ok_or_else(|| serde::de::Error::custom("Expected boolean value, got null"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Debug, Deserialize, PartialEq)]
    struct TestStruct {
        #[serde(default, deserialize_with = "deserialize_flexible_bool")]
        optional_bool: Option<bool>,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct TestStructRequired {
        #[serde(deserialize_with = "deserialize_flexible_bool_required")]
        required_bool: bool,
    }

    #[test]
    fn test_deserialize_actual_boolean_true() {
        let json = json!({ "optional_bool": true });
        let result: TestStruct = serde_json::from_value(json).unwrap();
        assert_eq!(result.optional_bool, Some(true));
    }

    #[test]
    fn test_deserialize_actual_boolean_false() {
        let json = json!({ "optional_bool": false });
        let result: TestStruct = serde_json::from_value(json).unwrap();
        assert_eq!(result.optional_bool, Some(false));
    }

    #[test]
    fn test_deserialize_string_true_lowercase() {
        let json = json!({ "optional_bool": "true" });
        let result: TestStruct = serde_json::from_value(json).unwrap();
        assert_eq!(result.optional_bool, Some(true));
    }

    #[test]
    fn test_deserialize_string_true_uppercase() {
        let json = json!({ "optional_bool": "TRUE" });
        let result: TestStruct = serde_json::from_value(json).unwrap();
        assert_eq!(result.optional_bool, Some(true));
    }

    #[test]
    fn test_deserialize_string_true_mixed_case() {
        let json = json!({ "optional_bool": "True" });
        let result: TestStruct = serde_json::from_value(json).unwrap();
        assert_eq!(result.optional_bool, Some(true));
    }

    #[test]
    fn test_deserialize_string_false_variations() {
        let cases = vec!["false", "False", "FALSE", "FaLsE"];
        for case in cases {
            let json = json!({ "optional_bool": case });
            let result: TestStruct = serde_json::from_value(json).unwrap();
            assert_eq!(
                result.optional_bool,
                Some(false),
                "Failed for case: {}",
                case
            );
        }
    }

    #[test]
    fn test_deserialize_yes_no_variations() {
        let true_cases = vec!["yes", "Yes", "YES"];
        for case in true_cases {
            let json = json!({ "optional_bool": case });
            let result: TestStruct = serde_json::from_value(json).unwrap();
            assert_eq!(
                result.optional_bool,
                Some(true),
                "Failed for case: {}",
                case
            );
        }

        let false_cases = vec!["no", "No", "NO"];
        for case in false_cases {
            let json = json!({ "optional_bool": case });
            let result: TestStruct = serde_json::from_value(json).unwrap();
            assert_eq!(
                result.optional_bool,
                Some(false),
                "Failed for case: {}",
                case
            );
        }
    }

    #[test]
    fn test_deserialize_numeric_strings() {
        let json = json!({ "optional_bool": "1" });
        let result: TestStruct = serde_json::from_value(json).unwrap();
        assert_eq!(result.optional_bool, Some(true));

        let json = json!({ "optional_bool": "0" });
        let result: TestStruct = serde_json::from_value(json).unwrap();
        assert_eq!(result.optional_bool, Some(false));
    }

    #[test]
    fn test_deserialize_null() {
        let json = json!({ "optional_bool": null });
        let result: TestStruct = serde_json::from_value(json).unwrap();
        assert_eq!(result.optional_bool, None);
    }

    #[test]
    fn test_deserialize_invalid_string() {
        let json = json!({ "optional_bool": "maybe" });
        let result: Result<TestStruct, _> = serde_json::from_value(json);
        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(error.contains("Invalid boolean string"));
        assert!(error.contains("maybe"));
    }

    #[test]
    fn test_deserialize_invalid_numeric_string() {
        let json = json!({ "optional_bool": "2" });
        let result: Result<TestStruct, _> = serde_json::from_value(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_required_bool_with_valid_value() {
        let json = json!({ "required_bool": true });
        let result: TestStructRequired = serde_json::from_value(json).unwrap();
        assert_eq!(result.required_bool, true);

        let json = json!({ "required_bool": "false" });
        let result: TestStructRequired = serde_json::from_value(json).unwrap();
        assert_eq!(result.required_bool, false);
    }

    #[test]
    fn test_required_bool_with_null_fails() {
        let json = json!({ "required_bool": null });
        let result: Result<TestStructRequired, _> = serde_json::from_value(json);
        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(error.contains("Expected boolean value, got null"));
    }

    #[test]
    fn test_edge_cases() {
        // Test with extra whitespace (should fail as we don't trim)
        let json = json!({ "optional_bool": " true " });
        let result: Result<TestStruct, _> = serde_json::from_value(json);
        assert!(result.is_err());

        // Test empty string (should fail)
        let json = json!({ "optional_bool": "" });
        let result: Result<TestStruct, _> = serde_json::from_value(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_generate_id() {
        // Test that generate_id produces strings of the correct length
        let id = generate_id(10);
        assert_eq!(id.len(), 10);

        // Test that all characters are alphanumeric
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));

        // Test different lengths
        let id = generate_id(6);
        assert_eq!(id.len(), 6);

        let id = generate_id(20);
        assert_eq!(id.len(), 20);

        // Test that consecutive calls produce different IDs (with high probability)
        let id1 = generate_id(10);
        let id2 = generate_id(10);
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_missing_field_handling() {
        // Test that our deserializer handles missing fields correctly
        let json = json!({ "optional_bool": null });
        let result: TestStruct = serde_json::from_value(json).unwrap();
        assert_eq!(result.optional_bool, None);

        // Test completely missing field
        let json = json!({});
        let result: TestStruct = serde_json::from_value(json).unwrap();
        assert_eq!(result.optional_bool, None);
    }
}
