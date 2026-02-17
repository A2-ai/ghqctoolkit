use anyhow::Result;
use http::StatusCode;
use serde_json::Value;
use std::collections::HashMap;

use super::types::{ExpectedResponse, MatchType, SchemaType};

/// Response validation error
#[derive(Debug)]
pub struct ValidationError {
    pub message: String,
    pub details: Vec<String>,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        for detail in &self.details {
            write!(f, "\n    {}", detail)?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationError {}

/// Validates HTTP responses against expected results
pub struct ResponseAsserter<'a> {
    expected: &'a ExpectedResponse,
}

impl<'a> ResponseAsserter<'a> {
    pub fn new(expected: &'a ExpectedResponse) -> Self {
        Self { expected }
    }

    /// Validate the response status and body
    pub fn validate(
        &self,
        actual_status: StatusCode,
        actual_body: Option<&Value>,
    ) -> Result<(), ValidationError> {
        let mut errors = Vec::new();

        // Check status code
        let expected_status =
            StatusCode::from_u16(self.expected.status).map_err(|_| ValidationError {
                message: format!("Invalid expected status code: {}", self.expected.status),
                details: vec![],
            })?;

        if actual_status != expected_status {
            errors.push(format!(
                "Status code mismatch: expected {}, got {}",
                expected_status, actual_status
            ));
        }

        // Check body if specified
        if let Some(expected_body) = &self.expected.body {
            match actual_body {
                Some(body) => {
                    match expected_body.match_type {
                        MatchType::Exact => {
                            // For exact match, use 'value' field if present, otherwise use 'fields'
                            let expected_value = if let Some(ref val) = expected_body.value {
                                // Direct value specified (for arrays, scalars, or complete objects)
                                val.clone()
                            } else if !expected_body.fields.is_empty() {
                                // Fields specified - construct an object
                                Value::Object(
                                    expected_body
                                        .fields
                                        .iter()
                                        .map(|(k, v)| (k.clone(), v.clone()))
                                        .collect(),
                                )
                            } else {
                                // Neither value nor fields specified
                                errors.push("Exact match requires either 'value' or 'fields' to be specified".to_string());
                                return if errors.is_empty() {
                                    Ok(())
                                } else {
                                    Err(ValidationError {
                                        message: "Response validation failed".to_string(),
                                        details: errors,
                                    })
                                };
                            };
                            if let Err(e) = self.validate_exact(body, &expected_value) {
                                errors.push(e);
                            }
                        }
                        MatchType::Partial => {
                            if let Err(e) = self.validate_partial(body, &expected_body.fields) {
                                errors.push(e);
                            }
                        }
                        MatchType::Schema => match &expected_body.schema {
                            Some(schema) => {
                                if let Err(e) = self.validate_schema(body, schema) {
                                    errors.push(e);
                                }
                            }
                            None => {
                                errors.push(
                                    "Schema match_type requires schema field to be specified"
                                        .to_string(),
                                );
                            }
                        },
                    }
                }
                None => errors.push("Expected response body, but got none".to_string()),
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationError {
                message: "Response validation failed".to_string(),
                details: errors,
            })
        }
    }

    /// Validate exact match (full JSON equality)
    fn validate_exact(&self, actual: &Value, expected: &Value) -> Result<String, String> {
        if actual != expected {
            let actual_pretty = serde_json::to_string_pretty(actual).unwrap_or_default();
            let expected_pretty = serde_json::to_string_pretty(expected).unwrap_or_default();
            Err(format!(
                "Exact match failed:\nExpected:\n{}\n\nActual:\n{}",
                expected_pretty, actual_pretty
            ))
        } else {
            Ok(String::new())
        }
    }

    /// Validate partial match (check only specified fields)
    fn validate_partial(
        &self,
        actual: &Value,
        expected_fields: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let actual_obj = match actual {
            Value::Object(obj) => obj,
            _ => {
                return Err(format!(
                    "Expected object for partial match, got {}",
                    value_type_name(actual)
                ));
            }
        };

        let mut errors = Vec::new();
        for (key, expected_value) in expected_fields {
            match actual_obj.get(key) {
                Some(actual_value) => {
                    if actual_value != expected_value {
                        errors.push(format!(
                            "Field '{}': expected {}, got {}",
                            key,
                            serde_json::to_string(expected_value).unwrap_or_default(),
                            serde_json::to_string(actual_value).unwrap_or_default()
                        ));
                    }
                }
                None => errors.push(format!("Missing field: '{}'", key)),
            }
        }

        if errors.is_empty() {
            Ok(String::new())
        } else {
            Err(format!("Partial match failed:\n  {}", errors.join("\n  ")))
        }
    }

    /// Validate schema (structure and types)
    fn validate_schema(
        &self,
        actual: &Value,
        schema: &super::types::SchemaAssertion,
    ) -> Result<String, String> {
        let mut errors = Vec::new();

        // Check type
        let actual_type = match actual {
            Value::Object(_) => SchemaType::Object,
            Value::Array(_) => SchemaType::Array,
            Value::String(_) => SchemaType::String,
            Value::Number(_) => SchemaType::Number,
            Value::Bool(_) => SchemaType::Boolean,
            Value::Null => SchemaType::Null,
        };

        if actual_type != schema.schema_type {
            errors.push(format!(
                "Type mismatch: expected {:?}, got {:?}",
                schema.schema_type, actual_type
            ));
        }

        // For arrays, check min_length, exact_length, item_fields, and first_item
        if let Value::Array(arr) = actual {
            if let Some(min_len) = schema.min_length {
                if arr.len() < min_len {
                    errors.push(format!(
                        "Array too short: expected at least {} items, got {}",
                        min_len,
                        arr.len()
                    ));
                }
            }

            if let Some(exact_len) = schema.exact_length {
                if arr.len() != exact_len {
                    errors.push(format!(
                        "Array length mismatch: expected exactly {} items, got {}",
                        exact_len,
                        arr.len()
                    ));
                }
            }

            // Check item_fields if specified (all items should have these fields)
            if !schema.item_fields.is_empty() && !arr.is_empty() {
                for (idx, item) in arr.iter().enumerate() {
                    if let Value::Object(obj) = item {
                        for field in &schema.item_fields {
                            if !obj.contains_key(field) {
                                errors.push(format!(
                                    "Array item [{}] missing field: '{}'",
                                    idx, field
                                ));
                            }
                        }
                    } else {
                        errors.push(format!(
                            "Array item [{}] is not an object (cannot check fields)",
                            idx
                        ));
                    }
                }
            }

            // Check first_item field values if specified
            if let Some(first_item_expected) = &schema.first_item {
                if arr.is_empty() {
                    errors.push("Cannot validate first_item: array is empty".to_string());
                } else if let Value::Object(first_obj) = &arr[0] {
                    for (key, expected_value) in first_item_expected {
                        match first_obj.get(key) {
                            Some(actual_value) => {
                                // Support nested object validation
                                if let Err(e) =
                                    self.validate_value_match(actual_value, expected_value, key)
                                {
                                    errors.push(format!("First item field '{}': {}", key, e));
                                }
                            }
                            None => errors.push(format!("First item missing field: '{}'", key)),
                        }
                    }
                } else {
                    errors.push(
                        "First array item is not an object (cannot check fields)".to_string(),
                    );
                }
            }
        }

        // For objects, check required fields
        if let Value::Object(obj) = actual {
            for field in &schema.item_fields {
                if !obj.contains_key(field) {
                    errors.push(format!("Object missing field: '{}'", field));
                }
            }
        }

        if errors.is_empty() {
            Ok(String::new())
        } else {
            Err(format!(
                "Schema validation failed:\n  {}",
                errors.join("\n  ")
            ))
        }
    }

    /// Validate that actual_value matches expected_value
    /// Supports partial matching for nested objects
    fn validate_value_match(
        &self,
        actual: &Value,
        expected: &Value,
        field_path: &str,
    ) -> Result<(), String> {
        match (actual, expected) {
            // Both are objects: validate that expected fields exist and match in actual
            (Value::Object(actual_obj), Value::Object(expected_obj)) => {
                let mut errors = Vec::new();
                for (key, expected_val) in expected_obj {
                    match actual_obj.get(key) {
                        Some(actual_val) => {
                            let nested_path = format!("{}.{}", field_path, key);
                            if let Err(e) =
                                self.validate_value_match(actual_val, expected_val, &nested_path)
                            {
                                errors.push(e);
                            }
                        }
                        None => {
                            errors.push(format!("missing nested field '{}.{}'", field_path, key));
                        }
                    }
                }
                if errors.is_empty() {
                    Ok(())
                } else {
                    Err(errors.join(", "))
                }
            }
            // Both are arrays: validate array equality
            (Value::Array(actual_arr), Value::Array(expected_arr)) => {
                if actual_arr == expected_arr {
                    Ok(())
                } else {
                    Err(format!(
                        "array mismatch: expected {}, got {}",
                        serde_json::to_string(expected).unwrap_or_default(),
                        serde_json::to_string(actual).unwrap_or_default()
                    ))
                }
            }
            // Scalar values: exact equality
            _ => {
                if actual == expected {
                    Ok(())
                } else {
                    Err(format!(
                        "expected {}, got {}",
                        serde_json::to_string(expected).unwrap_or_default(),
                        serde_json::to_string(actual).unwrap_or_default()
                    ))
                }
            }
        }
    }
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
