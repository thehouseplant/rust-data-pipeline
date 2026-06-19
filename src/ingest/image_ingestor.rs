use super::{Ingestor, Record};
use crate::error::IngestError;
use serde_json::{json, Map, Value};
use std::io::Cursor;

pub struct ImageMetaIngestor;

impl Ingestor for ImageMetaIngestor {
    fn parse(&self, filename: &str, bytes: &[u8]) -> Result<Vec<Record>, IngestError> {
        let mut payload = Map::new();
        payload.insert("filename".to_string(), json!(filename));
        payload.insert("size_bytes".to_string(), json!(bytes.len()));

        // EXIF is optional — plenty of valid images (e.g. PNGs, screenshots)
        // simply won't have any. We don't fail the upload if it's absent,
        // we just record what we found.
        let mut cursor = Cursor::new(bytes);
        match exif::Reader::new().read_from_container(&mut cursor) {
            Ok(exif_data) => {
                let mut fields = Map::new();
                for field in exif_data.fields() {
                    fields.insert(
                        field.tag.to_string(),
                        json!(field.display_value().with_unit(&exif_data).to_string()),
                    );
                }
                payload.insert("exif".to_string(), Value::Object(fields));
            }
            Err(_) => {
                payload.insert("exif".to_string(), Value::Null);
            }
        }

        Ok(vec![Record {
            asset_type: "image_metadata",
            payload: Value::Object(payload),
            row_index: None,
        }])
    }
}
