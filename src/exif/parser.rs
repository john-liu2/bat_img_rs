//! EXIF parsing using `exif_lib`.

use crate::exif::container::{extract_exif_tiff, read_short_tag_from_tiff};
use crate::exif::extract_heic_exif_raw;
use crate::exif::heic::tiff_from_heic_metadata;
use crate::heic;
use exif_lib::{In, Reader, Tag};
use std::path::Path;

#[derive(Debug, Default)]
pub struct ExifInfo {
    pub make: Option<String>,
    pub model: Option<String>,
    pub date_time: Option<String>,
    pub iso: Option<String>,
    pub exposure: Option<String>,
    pub f_number: Option<String>,
    pub focal_length: Option<String>,
    pub gps_present: bool,
}

/// Read EXIF metadata from a file path (supporting JPEG, TIFF, and HEIC).
pub fn read_exif(path: &Path) -> Option<ExifInfo> {
    let raw_bytes = std::fs::read(path).ok()?;
    // 1. Try extracting from HEIC using libheif's native EXIF extraction
    if heic::is_heic_bytes(&raw_bytes)
        && let Some(heic_exif) = extract_heic_exif_native(&raw_bytes)
        && let Some(info) = parse_exif_bytes(&heic_exif)
    {
        return Some(info);
    }
    // 2. Try extracting using our custom container scanning
    let tiff_bytes = if let Some(extracted) = extract_heic_exif_raw(&raw_bytes) {
        extracted
    } else {
        raw_bytes.clone()
    };
    // 3. Attempt to parse the TIFF
    parse_exif_bytes(&tiff_bytes)
}

/// Parse EXIF from raw TIFF/EXIF bytes.
pub fn parse_exif_bytes(bytes: &[u8]) -> Option<ExifInfo> {
    let mut reader = Reader::new();
    reader.continue_on_error(true);
    let exif_data = match reader
        .read_raw(bytes.to_vec())
        .or_else(|error| error.distill_partial_result(|_| {}))
    {
        Ok(exif_data) => exif_data,
        Err(_) => {
            let mut reader = Reader::new();
            reader.continue_on_error(true);
            reader
                .read_from_container(&mut std::io::Cursor::new(bytes))
                .or_else(|error| error.distill_partial_result(|_| {}))
                .ok()?
        }
    };
    parse_exif_info(&exif_data)
}

/// Read the EXIF orientation tag (tag 0x0112) from any supported image container.
pub fn read_orientation(bytes: &[u8]) -> Option<u32> {
    let tiff = extract_exif_tiff(bytes)?;
    read_short_tag_from_tiff(&tiff, 0x0112).map(|v| v as u32)
}

// ---- internal -------------------------------------------------------------

fn parse_exif_info(exif_data: &exif_lib::Exif) -> Option<ExifInfo> {
    let mut info = ExifInfo::default();

    if let Some(field) = exif_data.get_field(Tag::Make, In::PRIMARY) {
        info.make = Some(
            field
                .display_value()
                .to_string()
                .trim_matches('"')
                .to_string(),
        );
    }
    if let Some(field) = exif_data.get_field(Tag::Model, In::PRIMARY) {
        info.model = Some(
            field
                .display_value()
                .to_string()
                .trim_matches('"')
                .to_string(),
        );
    }
    if let Some(field) = exif_data.get_field(Tag::DateTime, In::PRIMARY) {
        info.date_time = Some(
            field
                .display_value()
                .to_string()
                .trim_matches('"')
                .to_string(),
        );
    }
    if let Some(field) = exif_data.get_field(Tag::PhotographicSensitivity, In::PRIMARY) {
        info.iso = Some(field.display_value().to_string());
    }
    if let Some(field) = exif_data.get_field(Tag::ExposureTime, In::PRIMARY) {
        info.exposure = Some(field.display_value().to_string());
    }
    if let Some(field) = exif_data.get_field(Tag::FNumber, In::PRIMARY) {
        info.f_number = Some(field.display_value().to_string());
    }
    if let Some(field) = exif_data.get_field(Tag::FocalLength, In::PRIMARY) {
        info.focal_length = Some(field.display_value().to_string());
    }
    if exif_data.get_field(Tag::GPSLatitude, In::PRIMARY).is_some()
        || exif_data
            .get_field(Tag::GPSInfoIFDPointer, In::PRIMARY)
            .is_some()
        || exif_data
            .get_field(Tag::GPSVersionID, In::PRIMARY)
            .is_some()
    {
        info.gps_present = true;
    }
    Some(info)
}

fn extract_heic_exif_native(bytes: &[u8]) -> Option<Vec<u8>> {
    use libheif_rs::{HeifContext, ItemId};
    let ctx = HeifContext::read_from_bytes(bytes).ok()?;
    let handle = ctx.primary_image_handle().ok()?;
    let mut ids: Vec<ItemId> = vec![0; 32];
    let count = handle.metadata_block_ids(&mut ids, b"Exif");
    for &id in ids.iter().take(count) {
        if let Ok(data) = handle.metadata(id)
            && let Some(tiff) = tiff_from_heic_metadata(&data)
        {
            return Some(tiff.to_vec());
        }
    }
    None
}
