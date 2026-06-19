use crate::error::IngestError;
use serde_json::Value;

/// A single unit of parsed data, ready to be inserted as one row.
/// `asset_type` becomes the discriminator column; `payload` is stored as JSONB.
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
/// upload handler or the dispatch logic — just a new impl + one match arm.
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
    if let Some(kind) = infer::get(bytes) {
        let mime = kind.mime_type();
        if mime == "text/csv" {
            return Ok(Box::new(CsvIngestor));
        }
        if mime.starts_with("image/") {
            return Ok(Box::new(ImageMetaIngestor));
        }
    }

    // Last resort: does it parse as JSON at all?
    if serde_json::from_slice::<Value>(bytes).is_ok() {
        return Ok(Box::new(JsonIngestor));
    }

    Err(IngestError::Malformed(format!(
        "Could not determine asset type for file '{filename}'"
    )))
}
