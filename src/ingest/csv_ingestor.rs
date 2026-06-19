use super::{Ingestor, Record};
use crate::error::IngestError;
use serde_json::{Map, Value};

pub struct CsvIngestor;

impl Ingestor for CsvIngestor {
    fn parse(&self, filename: &str, bytes: &[u8]) -> Result<Vec<Record>, IngestError> {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_reader(bytes);

        let headers = reader
            .headers()
            .map_err(|e| IngestError::Malformed(format!("'{filename}': bad CSV headers: {e}")))?
            .clone();

        let mut records = Vec::new();

        for (idx, result) in reader.records().enumerate() {
            let row = result
                .map_err(|e| IngestError::Malformed(format!("'{filename}' row {idx}: {e}")))?;

            let mut obj = Map::new();
            for (header, value) in headers.iter().zip(row.iter()) {
                obj.insert(header.to_string(), Value::String(value.to_string()));
            }

            records.push(Record {
                asset_type: "csv_row",
                payload: Value::Object(obj),
                row_index: Some(idx as i32),
            });
        }

        if records.is_empty() {
            return Err(IngestError::Malformed(format!(
                "'{filename}' contained no data rows"
            )));
        }

        Ok(records)
    }
}
