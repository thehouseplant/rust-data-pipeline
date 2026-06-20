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

        // EXIF is optional - plenty of valid images (e.g. PNGs, screenshots)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garbage_bytes_never_panic_and_still_return_a_record() {
        // This is the core safety property of this ingestor: parse()
        // never returns Err, and never panics, regardless of input,
        // because EXIF is treated as optional metadata rather than
        // something that gates a successful upload.
        let garbage = b"this is not an image at all, just some bytes";
        let records = ImageMetaIngestor.parse("not-an-image.jpg", garbage).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].asset_type, "image_metadata");
        assert_eq!(records[0].payload["exif"], Value::Null);
        assert_eq!(records[0].payload["size_bytes"], garbage.len());
        assert_eq!(records[0].payload["filename"], "not-an-image.jpg");
    }

    #[test]
    fn empty_byte_input_does_not_panic() {
        let records = ImageMetaIngestor.parse("empty.jpg", b"").unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].payload["exif"], Value::Null);
        assert_eq!(records[0].payload["size_bytes"], 0);
    }

    #[test]
    fn filename_is_recorded_exactly_as_given() {
        let records = ImageMetaIngestor
            .parse("vacation photo (final) v2.JPG", b"\x00\x01\x02")
            .unwrap();

        assert_eq!(
            records[0].payload["filename"],
            "vacation photo (final) v2.JPG"
        );
    }

    // NOTE: this test requires a real JPEG with embedded EXIF data and is
    // ignored by default since no such fixture ships in this repo.
    //
    // To enable it:
    //   1. Drop a real photo with EXIF data at tests/fixtures/with_exif.jpg
    //      (e.g. an unedited photo straight off a phone or camera - most
    //      photo editing tools and screenshot tools strip EXIF on save).
    //   2. Run: cargo test -- --ignored
    //
    // We don't ship a binary fixture by default to keep the repo lean and
    // avoid committing binary blobs whose provenance isn't obvious - but
    // this path (an image that DOES have real, parseable EXIF) is exactly
    // the one most worth verifying by hand once, since it's the path the
    // synthetic tests above can't exercise.
    #[test]
    #[ignore]
    fn real_jpeg_with_exif_populates_exif_fields() {
        let bytes = std::fs::read("tests/fixtures/with_exif.jpg")
            .expect("place a real EXIF-bearing JPEG at tests/fixtures/with_exif.jpg to run this test");

        let records = ImageMetaIngestor.parse("with_exif.jpg", &bytes).unwrap();

        assert_eq!(records.len(), 1);
        assert_ne!(
            records[0].payload["exif"],
            Value::Null,
            "expected EXIF data to be present and parsed"
        );
    }
}
