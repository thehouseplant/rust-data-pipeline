use super::{Ingestor, Record};
use crate::error::IngestError;
use serde_json::Value;

pub struct JsonIngestor;

impl Ingestor for JsonIngestor {
    fn parse(&self, filename: &str, bytes: &[u8]) -> Result<Vec<Record>, IngestError> {
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|e| IngestError::Malformed(format!("'{filename}': invalid JSON: {e}")))?;

        // If the top-level value is an array, treat each element as its own row.
        // Otherwise treat the whole document as a single record.
        let records = match value {
            Value::Array(items) => items
                .into_iter()
                .enumerate()
                .map(|(idx, item)| Record {
                    asset_type: "json",
                    payload: item,
                    row_index: Some(idx as i32),
                })
                .collect(),
            other => vec![Record {
                asset_type: "json",
                payload: other,
                row_index: None,
            }],
        };

        if records.is_empty() {
            return Err(IngestError::Malformed(format!(
                "'{filename}' contained an empty array"
            )));
        }

        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_input_yields_one_record_per_element() {
        let json = br#"[{"id": 1}, {"id": 2}, {"id": 3}]"#;
        let records = JsonIngestor.parse("data.json", json).unwrap();

        assert_eq!(records.len(), 3);
        assert_eq!(records[0].row_index, Some(0));
        assert_eq!(records[2].row_index, Some(2));
        assert_eq!(records[1].payload["id"], 2);
        assert!(records.iter().all(|r| r.asset_type == "json"));
    }

    #[test]
    fn single_object_input_yields_one_record_with_no_row_index() {
        let json = br#"{"event": "signup", "user_id": 42}"#;
        let records = JsonIngestor.parse("event.json", json).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].row_index, None);
        assert_eq!(records[0].payload["event"], "signup");
        assert_eq!(records[0].payload["user_id"], 42);
    }

    #[test]
    fn scalar_top_level_value_is_still_a_single_record() {
        // Top-level value is just a number, not an object or array.
        // Document/confirm this doesn't panic and wraps it as-is.
        let json = b"42";
        let records = JsonIngestor.parse("scalar.json", json).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].payload, 42);
    }

    #[test]
    fn empty_array_is_an_error() {
        let result = JsonIngestor.parse("empty.json", b"[]");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("empty array"), "unexpected error: {err}");
    }

    #[test]
    fn malformed_json_is_an_error_not_a_panic() {
        let result = JsonIngestor.parse("broken.json", b"{not valid json");
        assert!(result.is_err());
    }

    #[test]
    fn empty_file_is_an_error() {
        let result = JsonIngestor.parse("empty.json", b"");
        assert!(result.is_err());
    }

    #[test]
    fn nested_objects_and_arrays_are_preserved_as_is() {
        let json = br#"{"user": {"name": "Alice", "tags": ["admin", "beta"]}}"#;
        let records = JsonIngestor.parse("nested.json", json).unwrap();

        assert_eq!(records[0].payload["user"]["name"], "Alice");
        assert_eq!(records[0].payload["user"]["tags"][0], "admin");
        assert_eq!(records[0].payload["user"]["tags"][1], "beta");
    }

    #[test]
    fn array_of_heterogeneous_shapes_does_not_error() {
        // Our JSONB-backed storage model tolerates this; worth a regression
        // test in case someone "fixes" this into validating a fixed schema
        // later without realizing mixed shapes were an intentional choice.
        let json = br#"[{"a": 1}, {"b": "two"}, [1, 2, 3], "plain string", null]"#;
        let records = JsonIngestor.parse("mixed.json", json).unwrap();

        assert_eq!(records.len(), 5);
        assert_eq!(records[3].payload, "plain string");
        assert_eq!(records[4].payload, serde_json::Value::Null);
    }
}
