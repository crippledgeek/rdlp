//! Evaluation of match filter conditions against JSON values.

use serde_json::Value;

use super::{ComparisonOp, Condition, FilterValue};

/// Evaluate a single condition against a JSON object.
pub(super) fn evaluate_condition(condition: &Condition, data: &Value) -> bool {
    match condition {
        Condition::Unary { key, negated } => {
            let value = data.get(key);
            let present = match value {
                None | Some(Value::Null) => false,
                Some(Value::Bool(b)) => *b,
                Some(_) => true,
            };
            if *negated { !present } else { present }
        }
        Condition::Comparison {
            key,
            op,
            negated,
            none_inclusive,
            value,
        } => {
            let actual = data.get(key);
            match actual {
                None | Some(Value::Null) => {
                    // Field missing — pass if none_inclusive, fail otherwise
                    *none_inclusive
                }
                Some(actual) => {
                    let result = evaluate_comparison(*op, actual, value);
                    if *negated { !result } else { result }
                }
            }
        }
    }
}

/// Evaluate a comparison between an actual JSON value and a filter value.
fn evaluate_comparison(op: ComparisonOp, actual: &Value, filter_val: &FilterValue) -> bool {
    match op {
        // Numeric/string comparison operators
        ComparisonOp::Lt | ComparisonOp::Le | ComparisonOp::Gt | ComparisonOp::Ge => {
            if let Some(actual_num) = as_number(actual)
                && let FilterValue::Number(filter_num) = filter_val
            {
                return match op {
                    ComparisonOp::Lt => actual_num < *filter_num,
                    ComparisonOp::Le => actual_num <= *filter_num,
                    ComparisonOp::Gt => actual_num > *filter_num,
                    ComparisonOp::Ge => actual_num >= *filter_num,
                    _ => unreachable!(),
                };
            }
            // String comparison fallback
            let actual_str = as_string(actual);
            let filter_str = filter_value_as_string(filter_val);
            match op {
                ComparisonOp::Lt => actual_str < filter_str,
                ComparisonOp::Le => actual_str <= filter_str,
                ComparisonOp::Gt => actual_str > filter_str,
                ComparisonOp::Ge => actual_str >= filter_str,
                _ => unreachable!(),
            }
        }

        // Equality
        ComparisonOp::Eq | ComparisonOp::Ne => {
            let equal = as_number(actual).map_or_else(
                || as_string(actual) == filter_value_as_string(filter_val),
                |actual_num| {
                    if let FilterValue::Number(filter_num) = filter_val {
                        (actual_num - *filter_num).abs() < f64::EPSILON
                    } else {
                        as_string(actual) == filter_value_as_string(filter_val)
                    }
                },
            );
            match op {
                ComparisonOp::Eq => equal,
                ComparisonOp::Ne => !equal,
                _ => unreachable!(),
            }
        }

        // String-only operators
        ComparisonOp::Contains => {
            let actual_str = as_string(actual);
            let filter_str = filter_value_as_string(filter_val);
            actual_str.contains(&filter_str)
        }
        ComparisonOp::StartsWith => {
            let actual_str = as_string(actual);
            let filter_str = filter_value_as_string(filter_val);
            actual_str.starts_with(&filter_str)
        }
        ComparisonOp::EndsWith => {
            let actual_str = as_string(actual);
            let filter_str = filter_value_as_string(filter_val);
            actual_str.ends_with(&filter_str)
        }
        ComparisonOp::Regex => {
            let actual_str = as_string(actual);
            let filter_str = filter_value_as_string(filter_val);
            regex::Regex::new(&filter_str).is_ok_and(|re| re.is_match(&actual_str))
        }
    }
}

/// Extract a numeric value from a JSON value.
fn as_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        // Also try parsing string fields as numbers (e.g., "1200")
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// Convert a JSON value to a string for comparison.
fn as_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        _ => value.to_string(),
    }
}

/// Convert a filter value to a string.
fn filter_value_as_string(value: &FilterValue) -> String {
    match value {
        FilterValue::Number(n) => n.to_string(),
        FilterValue::String(s) => s.clone(),
    }
}
