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
