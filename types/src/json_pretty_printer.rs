extern crate alloc;

use alloc::{format, string::String, vec::Vec};

use serde::Serialize;
use serde_json::{json, Value};

const MAX_STRING_LEN: usize = 150;

/// Serializes the given data structure as a pretty-printed `String` of JSON using
/// `serde_json::to_string_pretty()`, but after first reducing any large hex-string values.
///
/// A large hex-string is one containing only hex characters and which is over `MAX_STRING_LEN`.
/// Such hex-strings will be replaced by an indication of the number of chars redacted, for example
/// `[130 hex chars]`.
pub fn json_pretty_print<T>(value: &T) -> serde_json::Result<String>
where
    T: ?Sized + Serialize,
{
    let mut json_value = json!(value);
    shorten_string_field(&mut json_value);

    serde_json::to_string_pretty(&json_value)
}

fn shorten_string_field(value: &mut Value) {
    match value {
        Value::String(string) => {
            let mut ranges_to_remove = Vec::new();
            let mut start_index = 0;
            let mut contiguous_hex_count = 0;

            // Record all large hex-strings' start positions and lengths.
            for (index, char) in string.char_indices() {
                if char.is_ascii_hexdigit() {
                    if contiguous_hex_count == 0 {
                        // This is the start of a new hex-string.
                        start_index = index;
                    }
                    contiguous_hex_count += 1;
                } else if contiguous_hex_count != 0 {
                    // This is the end of a hex-string: if it's too long, record it.
                    if contiguous_hex_count > MAX_STRING_LEN {
                        ranges_to_remove.push((start_index, contiguous_hex_count));
                    }
                    contiguous_hex_count = 0;
                }
            }
            // If the string contains a large hex-string at the end, record it now.
            if contiguous_hex_count > MAX_STRING_LEN {
                ranges_to_remove.push((start_index, contiguous_hex_count));
            }

            // Replace the recorded large hex-strings.  Iterate from last to first so each
            // replacement start index remains valid.
            for (start_index, contiguous_hex_count) in ranges_to_remove.into_iter().rev() {
                let range = start_index..(start_index + contiguous_hex_count);
                string.replace_range(range, &format!("[{} hex chars]", contiguous_hex_count));
            }
        }
        Value::Array(values) => {
            for value in values {
                shorten_string_field(value);
            }
        }
        Value::Object(map) => {
            for map_value in map.values_mut() {
                shorten_string_field(map_value);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_string(length: usize) -> String {
        "0123456789abcdef".chars().cycle().take(length).collect()
    }

    #[test]
    fn should_shorten_long_strings() {
        let max_unshortened_hex_string = hex_string(MAX_STRING_LEN);
        let long_hex_string = hex_string(MAX_STRING_LEN + 1);
        let long_non_hex_string: String = "g".repeat(MAX_STRING_LEN + 1);
        let long_hex_substring = format!("a-{}-b", hex_string(MAX_STRING_LEN + 1));
        let multiple_long_hex_substrings =
            format!("a: {0}, b: {0}, c: {0}", hex_string(MAX_STRING_LEN + 1));

        let mut long_strings: Vec<String> = vec![];
        for i in 1..=5 {
            long_strings.push("a".repeat(MAX_STRING_LEN + i));
        }
        let value = json!({
            "field_1": Option::<usize>::None,
            "field_2": true,
            "field_3": 123,
            "field_4": max_unshortened_hex_string,
            "field_5": ["short string value", long_hex_string],
            "field_6": {
                "f1": Option::<usize>::None,
                "f2": false,
                "f3": -123,
                "f4": long_non_hex_string,
                "f5": ["short string value", long_hex_substring],
                "f6": {
                    "final long string": multiple_long_hex_substrings
                }
            }
        });

        let expected = r#"{
  "field_1": null,
  "field_2": true,
  "field_3": 123,
  "field_4": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123",
  "field_5": [
    "short string value",
    "[101 hex chars]"
  ],
  "field_6": {
    "f1": null,
    "f2": false,
    "f3": -123,
    "f4": "ggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg",
    "f5": [
      "short string value",
      "a-[101 hex chars]-b"
    ],
    "f6": {
      "final long string": "a: [101 hex chars], b: [101 hex chars], c: [101 hex chars]"
    }
  }
}"#;

        let output = json_pretty_print(&value).unwrap();
        assert_eq!(
            output, expected,
            "Actual:\n{}\nExpected:\n{}\n",
            output, expected
        );
    }

    #[test]
    fn should_not_modify_short_strings() {
        let max_string: String = "a".repeat(MAX_STRING_LEN);
        let value = json!({
            "field_1": Option::<usize>::None,
            "field_2": true,
            "field_3": 123,
            "field_4": max_string,
            "field_5": [
                "short string value",
                "another short string"
            ],
            "field_6": {
                "f1": Option::<usize>::None,
                "f2": false,
                "f3": -123,
                "f4": "short",
                "f5": [
                    "short string value",
                    "another short string"
                ],
                "f6": {
                    "final string": "the last short string"
                }
            }
        });

        let expected = serde_json::to_string_pretty(&value).unwrap();
        let output = json_pretty_print(&value).unwrap();
        assert_eq!(
            output, expected,
            "Actual:\n{}\nExpected:\n{}\n",
            output, expected
        );
    }

    #[test]
    /// Ref: https://github.com/casper-network/casper-node/issues/1456
    fn regression_1456() {
        let long_string = r#"state query failed: ValueNotFound("Failed to find base key at path: Key::Account(72698d4dc715a28347b15920b09b4f0f1d633be5a33f4686d06992415b0825e2)")"#;
        assert_eq!(long_string.len(), 148);

        let value = json!({
            "code": -32003,
            "message": long_string,
        });

        let expected = r#"{
  "code": -32003,
  "message": "state query failed: ValueNotFound(\"Failed to find base key at path: Key::Account(72698d4dc715a28347b15920b09b4f0f1d633be5a33f4686d06992415b0825e2)\")"
}"#;

        let output = json_pretty_print(&value).unwrap();
        assert_eq!(
            output, expected,
            "Actual:\n{}\nExpected:\n{}\n",
            output, expected
        );
    }
}
