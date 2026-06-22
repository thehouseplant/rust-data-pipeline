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
            let row = result.map_err(|e| {
                IngestError::Malformed(format!("'{filename}' row {idx}: {e}"))
            })?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_csv_into_one_record_per_row() {
        let csv = "name,age,city\nAlice,30,Seattle\nBob,25,Austin\n";
        let records = CsvIngestor.parse("people.csv", csv.as_bytes()).unwrap();

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].asset_type, "csv_row");
        assert_eq!(records[0].row_index, Some(0));
        assert_eq!(records[1].row_index, Some(1));

        assert_eq!(records[0].payload["name"], "Alice");
        assert_eq!(records[0].payload["age"], "30");
        assert_eq!(records[0].payload["city"], "Seattle");
        assert_eq!(records[1].payload["name"], "Bob");
    }

    #[test]
    fn header_only_csv_with_no_data_rows_is_an_error() {
        let csv = "name,age,city\n";
        let result = CsvIngestor.parse("empty.csv", csv.as_bytes());

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no data rows"), "unexpected error: {err}");
    }

    #[test]
    fn completely_empty_file_is_an_error() {
        let result = CsvIngestor.parse("empty.csv", b"");
        assert!(result.is_err());
    }

    #[test]
    fn row_with_fewer_fields_than_headers_does_not_panic() {
        // csv crate's default behavior is strict about field count unless
        // flexible() is set; we want to confirm this surfaces as our
        // IngestError rather than panicking or silently truncating data.
        let csv = "name,age,city\nAlice,30\n";
        let result = CsvIngestor.parse("ragged.csv", csv.as_bytes());

        assert!(result.is_err());
    }

    #[test]
    fn row_with_more_fields_than_headers_is_an_error() {
        let csv = "name,age\nAlice,30,extra_field\n";
        let result = CsvIngestor.parse("ragged.csv", csv.as_bytes());

        assert!(result.is_err());
    }

    #[test]
    fn values_containing_commas_are_handled_via_quoting() {
        let csv = "name,address\n\"Alice\",\"123 Main St, Apt 4\"\n";
        let records = CsvIngestor.parse("quoted.csv", csv.as_bytes()).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].payload["address"], "123 Main St, Apt 4");
    }

    #[test]
    fn unicode_values_are_preserved() {
        let csv = "name,city\n\u{00c9}lodie,M\u{00fc}nchen\n";
        let records = CsvIngestor.parse("unicode.csv", csv.as_bytes()).unwrap();

        assert_eq!(records[0].payload["name"], "\u{00c9}lodie");
        assert_eq!(records[0].payload["city"], "M\u{00fc}nchen");
    }

    #[test]
    fn empty_field_values_become_empty_strings_not_null() {
        let csv = "name,nickname\nAlice,\n";
        let records = CsvIngestor.parse("blank.csv", csv.as_bytes()).unwrap();

        // Worth documenting/asserting this explicitly: empty CSV cells are
        // empty strings in the JSON payload, not JSON null. Downstream
        // consumers should not assume absence == null here.
        assert_eq!(records[0].payload["nickname"], "");
    }
}
