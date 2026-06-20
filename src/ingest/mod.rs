use crate::error::IngestError;
use serde_json::Value;

/// A single unit of parsed data, ready to be inserted as one row.
/// `asset_type` becomes the discriminator column; `payload` is stored as JSONB.
#[derive(Debug)]
pub struct Record {
    pub asset_type: &'static str,
    pub payload: Value,
    pub row_index: Option<i32>,
}

/// Implemented by every per-format parser. Each Ingestor turns raw bytes
/// from an uploaded file into a list of Records ready for insertion.
///
/// Keeping this trait narrow (one method, plain bytes in / records out)
/// means adding a new asset type later never requires touching the
/// upload handler or the dispatch logic - just a new impl + one match arm.
pub trait Ingestor: Send + Sync {
    fn parse(&self, filename: &str, bytes: &[u8]) -> Result<Vec<Record>, IngestError>;
}

pub mod csv_ingestor;
pub mod image_ingestor;
pub mod json_ingestor;

pub use csv_ingestor::CsvIngestor;
pub use image_ingestor::ImageMetaIngestor;
pub use json_ingestor::JsonIngestor;

/// Resolves which Ingestor to use for a given upload.
/// Tries file extension first (cheap, usually correct), then falls back
/// to magic-byte content sniffing for ambiguous or mislabeled files.
pub fn resolve_ingestor(filename: &str, bytes: &[u8]) -> Result<Box<dyn Ingestor>, IngestError> {
    let ext = filename
        .rsplit('.')
        .next()
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "csv" => return Ok(Box::new(CsvIngestor)),
        "json" => return Ok(Box::new(JsonIngestor)),
        "jpg" | "jpeg" | "png" | "tiff" | "tif" => return Ok(Box::new(ImageMetaIngestor)),
        _ => {}
    }

    // Extension was missing or unrecognized - sniff the actual bytes.
    // Note: `infer` only detects formats with binary magic-number
    // signatures (images, archives, etc.) - plain text formats like CSV
    // have no such signature, so we can't ask `infer` to identify CSV.
    if let Some(kind) = infer::get(bytes) {
        let mime = kind.mime_type();
        if mime.starts_with("image/") {
            return Ok(Box::new(ImageMetaIngestor));
        }
        // If infer recognizes the bytes as some other known binary format
        // (zip, pdf, etc.) that we don't have an ingestor for, don't fall
        // through to treating it as text - fail explicitly instead.
        return Err(IngestError::Malformed(format!(
            "file '{filename}' was detected as '{mime}', which has no ingestor"
        )));
    }

    // No binary signature matched. Try JSON first (it's syntactically
    // strict, so a false positive is unlikely), then fall back to CSV
    // for anything that's valid UTF-8 text, on the assumption that
    // unrecognized plain text uploaded to this endpoint is probably CSV.
    if serde_json::from_slice::<Value>(bytes).is_ok() {
        return Ok(Box::new(JsonIngestor));
    }

    if std::str::from_utf8(bytes).is_ok() {
        return Ok(Box::new(CsvIngestor));
    }

    Err(IngestError::Malformed(format!(
        "could not determine asset type for file '{filename}'"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_name(ingestor: &dyn Ingestor) -> &'static str {
        // Helper to assert *which* ingestor was selected without exposing
        // a downcast/Any dance in every test - we infer it by behavior
        // instead, using inputs that only one ingestor type would accept.
        let csv_probe = ingestor.parse("probe", b"a,b\n1,2\n");
        let json_probe = ingestor.parse("probe", b"{}");
        match (csv_probe.is_ok(), json_probe.is_ok()) {
            (true, false) => "csv",
            (false, true) => "json",
            _ => "image_or_other",
        }
    }

    #[test]
    fn dot_csv_extension_dispatches_to_csv_ingestor() {
        let ingestor = resolve_ingestor("data.csv", b"a,b\n1,2\n").unwrap();
        assert_eq!(type_name(ingestor.as_ref()), "csv");
    }

    #[test]
    fn dot_json_extension_dispatches_to_json_ingestor() {
        let ingestor = resolve_ingestor("data.json", b"{}").unwrap();
        assert_eq!(type_name(ingestor.as_ref()), "json");
    }

    #[test]
    fn extension_matching_is_case_insensitive() {
        let ingestor = resolve_ingestor("DATA.CSV", b"a,b\n1,2\n").unwrap();
        assert_eq!(type_name(ingestor.as_ref()), "csv");
    }

    #[test]
    fn image_extensions_dispatch_to_image_ingestor() {
        for ext in ["jpg", "jpeg", "png", "tiff", "tif"] {
            let filename = format!("photo.{ext}");
            let ingestor = resolve_ingestor(&filename, b"\xff\xd8\xff").unwrap();
            assert_eq!(
                type_name(ingestor.as_ref()),
                "image_or_other",
                "extension '{ext}' did not dispatch to image ingestor"
            );
        }
    }

    #[test]
    fn no_extension_with_json_content_falls_back_to_json_sniffing() {
        let ingestor = resolve_ingestor("noextension", br#"{"a": 1}"#).unwrap();
        assert_eq!(type_name(ingestor.as_ref()), "json");
    }

    #[test]
    fn unrecognized_extension_with_csv_like_content_falls_back_to_csv() {
        // .dat is not a known extension, and the content isn't valid JSON,
        // so this should fall through to the plain-text/CSV fallback.
        let ingestor = resolve_ingestor("export.dat", b"name,age\nAlice,30\n").unwrap();
        assert_eq!(type_name(ingestor.as_ref()), "csv");
    }

    #[test]
    fn real_png_magic_bytes_dispatch_to_image_ingestor_even_without_extension() {
        // PNG signature: 89 50 4E 47 0D 0A 1A 0A
        let png_magic = b"\x89PNG\r\n\x1a\n";
        let ingestor = resolve_ingestor("noextension", png_magic).unwrap();
        assert_eq!(type_name(ingestor.as_ref()), "image_or_other");
    }

    #[test]
    fn binary_format_infer_recognizes_but_we_dont_support_is_rejected() {
        // ZIP signature: 50 4B 03 04. We don't have a zip ingestor, and
        // critically, this must NOT silently fall through to being
        // treated as CSV/text - that would silently corrupt the data.
        let zip_magic = b"PK\x03\x04";
        let result = resolve_ingestor("archive.zip_renamed", zip_magic);
        assert!(
            result.is_err(),
            "a recognized-but-unsupported binary format should be rejected, not misrouted"
        );
    }

    #[test]
    fn truly_unidentifiable_binary_garbage_is_rejected() {
        let garbage = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0xFF, 0x10, 0x20];
        let result = resolve_ingestor("mystery", &garbage);
        assert!(result.is_err());
    }

    #[test]
    fn empty_bytes_with_no_extension_dispatches_but_then_fails_to_parse() {
        let result = resolve_ingestor("noextension", b"");
        // Empty input is valid UTF-8 (trivially) and not valid JSON, so
        // this currently falls through to CSV per our text fallback.
        // Document that explicitly here rather than leaving it implicit -
        // the CSV ingestor itself will then reject it as having no rows.
        assert!(result.is_ok());
        let ingestor = result.unwrap();
        assert!(ingestor.parse("noextension", b"").is_err());
    }
}
