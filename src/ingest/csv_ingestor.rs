use super::{Ingestor, Record};
use crate::error::IngestError;
use serde_json::{Map, Value};

pub struct CsvIngestor;

/// The inferred type for a CSV column, determined by scanning all values
/// in that column before converting any of them. Per-column (not per-cell)
/// so that a column of integers doesn't produce mixed types if one row
/// happens to parse as float - the whole column gets one consistent type.
#[derive(Debug, PartialEq)]
enum ColumnType {
    /// Every non-empty cell parsed cleanly as i64.
    Integer,
    /// Every non-empty cell parsed cleanly as f64 (but not all as i64).
    Float,
    /// At least one non-empty cell couldn't be parsed as a number.
    Text,
}

fn infer_column_type(values: &[&str]) -> ColumnType {
    let non_empty: Vec<&&str> = values.iter().filter(|v| !v.is_empty()).collect();

    // All-empty columns get Text - no signal to infer from.
    if non_empty.is_empty() {
        return ColumnType::Text;
    }

    if non_empty.iter().all(|v| v.parse::<i64>().is_ok()) {
        return ColumnType::Integer;
    }

    if non_empty.iter().all(|v| v.parse::<f64>().is_ok()) {
        return ColumnType::Float;
    }

    ColumnType::Text
}

fn coerce_cell(value: &str, col_type: &ColumnType) -> Value {
    // Empty cells become null regardless of column type - an absent value
    // is more accurately represented as SQL/JSON null than as 0, 0.0, or "".
    if value.is_empty() {
        return Value::Null;
    }

    match col_type {
        ColumnType::Integer => value
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(value.to_string())),
        ColumnType::Float => value
            .parse::<f64>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(value.to_string())),
        ColumnType::Text => Value::String(value.to_string()),
    }
}

impl Ingestor for CsvIngestor {
    fn parse(&self, filename: &str, bytes: &[u8]) -> Result<Vec<Record>, IngestError> {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_reader(bytes);

        let headers = reader
            .headers()
            .map_err(|e| IngestError::Malformed(format!("'{filename}': bad CSV headers: {e}")))?
            .clone();

        // Buffer all raw string rows first so we can do a column-type
        // inference pass before committing to any JSON conversions.
        let raw_rows: Vec<Vec<String>> = reader
            .records()
            .enumerate()
            .map(|(idx, result)| {
                result
                    .map(|row| row.iter().map(|v| v.to_string()).collect())
                    .map_err(|e| IngestError::Malformed(format!("'{filename}' row {idx}: {e}")))
            })
            .collect::<Result<_, _>>()?;

        if raw_rows.is_empty() {
            return Err(IngestError::Malformed(format!(
                "'{filename}' contained no data rows"
            )));
        }

        // Infer one ColumnType per column by scanning the full column.
        let col_types: Vec<ColumnType> = (0..headers.len())
            .map(|col_idx| {
                let column_values: Vec<&str> =
                    raw_rows.iter().map(|row| row[col_idx].as_str()).collect();
                infer_column_type(&column_values)
            })
            .collect();

        // Convert raw strings to JSON values using the inferred types.
        let records = raw_rows
            .iter()
            .enumerate()
            .map(|(row_idx, row)| {
                let mut obj = Map::new();
                for ((header, value), col_type) in
                    headers.iter().zip(row.iter()).zip(col_types.iter())
                {
                    obj.insert(header.to_string(), coerce_cell(value, col_type));
                }
                Record {
                    asset_type: "csv_row",
                    payload: Value::Object(obj),
                    row_index: Some(row_idx as i32),
                }
            })
            .collect();

        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Unit tests on the type inference helpers ---

    #[test]
    fn infer_integer_column() {
        assert_eq!(infer_column_type(&["1", "2", "42", "0", "-7"]), ColumnType::Integer);
    }

    #[test]
    fn infer_float_column() {
        // Has a decimal so not all parse as i64, but all parse as f64.
        assert_eq!(infer_column_type(&["1.5", "2.0", "3.14"]), ColumnType::Float);
    }

    #[test]
    fn infer_integer_column_ignores_empty_cells() {
        // An empty cell means "no value" - it shouldn't drag a numeric
        // column down to Text just because one row was blank.
        assert_eq!(infer_column_type(&["1", "", "3"]), ColumnType::Integer);
    }

    #[test]
    fn all_empty_column_infers_as_text() {
        assert_eq!(infer_column_type(&["", "", ""]), ColumnType::Text);
    }

    #[test]
    fn mixed_numeric_and_text_infers_as_text() {
        assert_eq!(infer_column_type(&["42", "N/A", "100"]), ColumnType::Text);
    }

    #[test]
    fn integers_do_not_widen_to_float() {
        // "1", "2", "3" all parse as i64 so the column is Integer, not Float,
        // even though they'd also parse as f64. Integer is the stricter type
        // and preserves round-trip fidelity (42 != 42.0 in JSON).
        assert_eq!(infer_column_type(&["1", "2", "3"]), ColumnType::Integer);
    }

    #[test]
    fn coerce_integer_cell() {
        assert_eq!(coerce_cell("42", &ColumnType::Integer), Value::from(42i64));
    }

    #[test]
    fn coerce_negative_integer_cell() {
        assert_eq!(coerce_cell("-7", &ColumnType::Integer), Value::from(-7i64));
    }

    #[test]
    fn coerce_float_cell() {
        assert_eq!(coerce_cell("3.14", &ColumnType::Float), Value::from(3.14f64));
    }

    #[test]
    fn coerce_empty_cell_is_null_regardless_of_column_type() {
        assert_eq!(coerce_cell("", &ColumnType::Integer), Value::Null);
        assert_eq!(coerce_cell("", &ColumnType::Float), Value::Null);
        assert_eq!(coerce_cell("", &ColumnType::Text), Value::Null);
    }

    #[test]
    fn coerce_text_cell_stays_string() {
        assert_eq!(
            coerce_cell("Seattle", &ColumnType::Text),
            Value::String("Seattle".to_string())
        );
    }

    // --- Integration-level tests on the full CsvIngestor::parse ---

    #[test]
    fn numeric_columns_are_coerced_not_stringified() {
        let csv = "name,age,city\nAlice,30,Seattle\nBob,25,Austin\n";
        let records = CsvIngestor.parse("people.csv", csv.as_bytes()).unwrap();

        assert_eq!(records.len(), 2);
        // name and city are Text - stay as strings
        assert_eq!(records[0].payload["name"], "Alice");
        assert_eq!(records[0].payload["city"], "Seattle");
        // age is all-integer - should be a JSON number, not the string "30"
        assert_eq!(records[0].payload["age"], 30);
        assert_eq!(records[1].payload["age"], 25);
    }

    #[test]
    fn mixed_column_stays_as_text() {
        // "status" has both numeric and text values - the whole column
        // should stay as strings, not partially coerce.
        let csv = "id,status\n1,active\n2,0\n3,inactive\n";
        let records = CsvIngestor.parse("mixed.csv", csv.as_bytes()).unwrap();

        assert_eq!(records[0].payload["id"], 1);   // pure integer column
        assert_eq!(records[0].payload["status"], "active");  // mixed -> text
        assert_eq!(records[1].payload["status"], "0");       // stays string
    }

    #[test]
    fn empty_cells_become_null() {
        let csv = "name,nickname\nAlice,\n";
        let records = CsvIngestor.parse("blank.csv", csv.as_bytes()).unwrap();

        // Empty cells are now null, not empty string - this is a deliberate
        // behaviour change from the original implementation.
        assert_eq!(records[0].payload["nickname"], Value::Null);
        assert_eq!(records[0].payload["name"], "Alice");
    }

    #[test]
    fn column_with_some_empty_cells_infers_type_from_non_empty() {
        // "score" has one blank - should still be treated as an integer column.
        let csv = "player,score\nAlice,100\nBob,\nCarol,85\n";
        let records = CsvIngestor.parse("scores.csv", csv.as_bytes()).unwrap();

        assert_eq!(records[0].payload["score"], 100);
        assert_eq!(records[1].payload["score"], Value::Null); // blank -> null
        assert_eq!(records[2].payload["score"], 85);
    }

    #[test]
    fn float_column_is_preserved_as_float() {
        let csv = "item,price\napple,0.99\nbanana,1.49\n";
        let records = CsvIngestor.parse("prices.csv", csv.as_bytes()).unwrap();

        assert_eq!(records[0].payload["price"], 0.99f64);
        assert_eq!(records[1].payload["price"], 1.49f64);
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
    fn row_with_fewer_fields_than_headers_is_an_error() {
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
}
