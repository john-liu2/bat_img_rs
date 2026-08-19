//! EXIF metadata manipulation for JPEG, PNG, TIFF, WebP, and JPEG 2000.
//!
//! GPS removal rewrites the embedded TIFF/EXIF block in-place where possible.
//! Orientation is read from the same EXIF block across all supported containers.

use anyhow::{Context, Result};
use std::path::Path;
use crate::heic;
use std::fs::File;
use std::io::BufReader;

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

/// Parse EXIF metadata from an image file using `exif_lib`
pub fn read_exif(path: &Path) -> Option<ExifInfo> {
    let file = File::open(path).ok()?;
    let mut bufreader = BufReader::new(file);

    let exifreader = exif_lib::Reader::new()
        .read_from_container(&mut bufreader)
        .ok()?;

    let mut info = ExifInfo::default();
    let mut has_fields = false;

    for field in exifreader.fields() {
        has_fields = true;
        let tag_name = field.tag.to_string();
        let value = field.display_value().with_unit(&exifreader).to_string();

        match tag_name.as_str() {
            "Make" => info.make = Some(value),
            "Model" => info.model = Some(value),
            "DateTime" | "DateTimeOriginal" => info.date_time = Some(value),
            "PhotographicSensitivity" | "ISOSpeedRatings" => info.iso = Some(value),
            "ExposureTime" => info.exposure = Some(value),
            "FNumber" => info.f_number = Some(value),
            "FocalLength" => info.focal_length = Some(value),
            tag if tag.starts_with("GPS") => info.gps_present = true,
            _ => {}
        }
    }
    if has_fields {
        Some(info)
    } else {
        None
    }
}

// ── Format signatures ─────────────────────────────────────────────────────────
const SOI: [u8; 2] = [0xFF, 0xD8]; // JPEG Start of Image
const EXIF_HEADER: &[u8] = b"Exif\x00\x00";
const PNG_SIG: [u8; 8] = *b"\x89PNG\r\n\x1a\n";

/// Read the EXIF orientation tag (tag 0x0112) from any supported image container.
pub fn read_orientation(bytes: &[u8]) -> Option<u32> {
    let tiff = extract_jpeg_exif_tiff(bytes)?;
    parse_orientation_from_ifd(tiff)
}

/// Fast byte-level GPS stripping across all supported image formats.
pub fn strip_gps_metadata(bytes: &[u8]) -> Result<Vec<u8>> {
    if is_jpeg(bytes) {
        rewrite_jpeg_exif_without_gps(bytes)
    } else if is_png(bytes) {
        rewrite_png_exif_without_gps(bytes)
    } else if is_webp(bytes) {
        rewrite_webp_exif_without_gps(bytes)
    } else if is_tiff(bytes) {
        strip_gps_from_tiff(bytes)
    } else if heic::is_heic_bytes(bytes) {
        rewrite_heic_exif_without_gps(bytes)
    } else {
        Ok(bytes.to_vec())
    }
}

/// Fast byte-level complete metadata stripping across all supported image formats.
pub fn strip_all_metadata(bytes: &[u8]) -> Result<Vec<u8>> {
    if is_jpeg(bytes) {
        // Strip APP1-APP15 metadata and COM comment segments, preserving APP0 (JFIF)
        Ok(strip_jpeg_app_segments(bytes, |marker| marker != 0xE0))
    } else if is_png(bytes) {
        // Strip metadata chunks: eXIf, tEXt, zTXt, iTXt, iCCP
        Ok(strip_png_metadata(bytes))
    } else if is_webp(bytes) {
        // Strip EXIF, XMP, and ICCP chunks from WebP RIFF stream
        Ok(strip_webp_metadata(bytes))
    } else if is_tiff(bytes) {
        // Zero out GPS and metadata IFD pointers in place
        strip_gps_from_tiff(bytes)
    } else if heic::is_heic_bytes(bytes) {
        strip_all_heic_metadata(bytes)
    } else {
        Ok(bytes.to_vec())
    }
}
// ── PNG Metadata Fast Path ───────────────────────────────────────────────────
fn strip_png_metadata(png: &[u8]) -> Vec<u8> {
    rewrite_png_chunks(png, |ctype, _| {
        !matches!(ctype, b"eXIf" | b"tEXt" | b"zTXt" | b"iTXt" | b"iCCP")
    })
}

// ── WebP Metadata Fast Path ──────────────────────────────────────────────────
fn strip_webp_metadata(webp: &[u8]) -> Vec<u8> {
    rebuild_webp_chunks(webp, |fourcc, payload| {
        if matches!(fourcc, b"EXIF" | b"XMP " | b"ICCP") {
            None
        } else {
            Some(payload.to_vec())
        }
    })
    .unwrap_or_else(|| webp.to_vec())
}

/// Extract the raw TIFF/EXIF block from any supported image container.
pub fn extract_exif_tiff(bytes: &[u8]) -> Option<Vec<u8>> {
    if is_jpeg(bytes) {
        extract_jpeg_exif_tiff(bytes).map(|t| t.to_vec())
    } else if is_png(bytes) {
        extract_png_exif_tiff(bytes)
    } else if is_webp(bytes) {
        extract_webp_exif_tiff(bytes)
    } else if is_tiff(bytes) {
        Some(bytes.to_vec())
    } else {
        None
    }
}

/// Copy non-GPS EXIF from a GPS-stripped source image into freshly encoded output bytes.
/// Orientation is reset to 1 because callers bake EXIF orientation into pixels before
/// re-encoding. Returns the original output unchanged when the source has no EXIF.
pub fn rewrite_exif_metadata(output: &[u8], source_stripped: &[u8]) -> Result<Vec<u8>> {
    let Some(mut exif_tiff) = extract_exif_tiff(source_stripped) else {
        return Ok(output.to_vec());
    };
    reset_orientation_in_tiff(&mut exif_tiff);

    if is_jpeg(output) {
        inject_exif_into_jpeg(output, &exif_tiff)
    } else if is_png(output) {
        inject_exif_into_png(output, &exif_tiff)
    } else if is_webp(output) {
        inject_exif_into_webp(output, &exif_tiff)
    } else if is_tiff(output) {
        // Re-encoded TIFF has no embedded EXIF IFD; write the stripped IFD back in full.
        Ok(exif_tiff)
    } else {
        Ok(output.to_vec())
    }
}

/// Graft GPS-stripped EXIF from `source_stripped` into an on-disk encoded output file.
pub fn write_exif_file(output_path: &Path, source_stripped: &[u8]) -> Result<()> {
    let encoded = std::fs::read(output_path)
        .with_context(|| format!("Cannot read {} for EXIF graft", output_path.display()))?;
    let grafted = rewrite_exif_metadata(&encoded, source_stripped)?;
    std::fs::write(output_path, grafted)
        .with_context(|| format!("Cannot write EXIF graft to {}", output_path.display()))?;
    Ok(())
}

pub fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&SOI)
}

pub fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&PNG_SIG)
}

pub fn is_tiff(bytes: &[u8]) -> bool {
    bytes.len() >= 4
        && matches!(&bytes[..4], b"II\x2A\x00" | b"MM\x00\x2A")
}

pub fn is_webp(bytes: &[u8]) -> bool {
    bytes.len() >= 12
        && &bytes[0..4] == b"RIFF"
        && &bytes[8..12] == b"WEBP"
}

// ── HEIC, JPG Helpers ──────────────────────────────────────────────────────────────
// Fast byte-level stripping for HEIC meta box without pixel re-encoding
fn rewrite_heic_exif_without_gps(bytes: &[u8]) -> Result<Vec<u8>> {
    // Extract raw EXIF payload from HEIC container, modify TIFF header in-place,
    // and write back to the ISOBMFF meta atom.
    if let Some(exif_tiff) = extract_heic_exif_raw(bytes) {
        let stripped_tiff = strip_gps_from_tiff(&exif_tiff)?;
        return replace_heic_exif_payload(bytes, &stripped_tiff);
    }
    Ok(bytes.to_vec())
}

/// Extract the raw TIFF payload of the EXIF block inside a HEIC ISOBMFF container.
pub fn extract_heic_exif_raw(bytes: &[u8]) -> Option<Vec<u8>> {
    let (meta_start, meta_end) = find_heic_meta_bounds(bytes)?;
    let meta_slice = &bytes[meta_start..meta_end];

    // Locate the TIFF header (II*\0 or MM\0*) inside the meta box payload
    let tiff_offset = find_tiff_header(meta_slice)?;
    let abs_tiff_offset = meta_start + tiff_offset;

    // Extract from TIFF header start to the end of the meta block slice
    let raw_tiff = &bytes[abs_tiff_offset..meta_end];
    Some(raw_tiff.to_vec())
}

/// Replace the EXIF TIFF payload inside a HEIC file in-place.
pub fn replace_heic_exif_payload(bytes: &[u8], new_tiff: &[u8]) -> Result<Vec<u8>> {
    let Some((meta_start, meta_end)) = find_heic_meta_bounds(bytes) else {
        return Ok(bytes.to_vec());
    };
    let meta_slice = &bytes[meta_start..meta_end];

    let Some(tiff_offset) = find_tiff_header(meta_slice) else {
        return Ok(bytes.to_vec());
    };

    let abs_tiff_offset = meta_start + tiff_offset;
    let mut out = bytes.to_vec();

    // In-place byte copy (strip_gps_from_tiff preserves exact buffer length)
    if abs_tiff_offset + new_tiff.len() <= out.len() {
        out[abs_tiff_offset..abs_tiff_offset + new_tiff.len()].copy_from_slice(new_tiff);
    }

    Ok(out)
}

// ── Private HEIC Helpers ─────────────────────────────────────────────────────

/// Walk top-level ISOBMFF boxes to locate the (start, end) payload bounds of the 'meta' box.
fn find_heic_meta_bounds(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut pos = 0;

    while pos + 8 <= bytes.len() {
        let mut box_size = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
        let box_type = &bytes[pos + 4..pos + 8];

        let header_len = if box_size == 1 {
            if pos + 16 > bytes.len() { break; }
            box_size = u64::from_be_bytes([
                bytes[pos + 8],  bytes[pos + 9],  bytes[pos + 10], bytes[pos + 11],
                bytes[pos + 12], bytes[pos + 13], bytes[pos + 14], bytes[pos + 15],
            ]) as usize;
            16
        } else if box_size == 0 {
            bytes.len() - pos
        } else {
            8
        };

        if box_size < header_len || pos.checked_add(box_size).map_or(true, |end| end > bytes.len()) {
            break;
        }

        if box_type == b"meta" {
            return Some((pos + header_len, pos + box_size));
        }

        pos += box_size;
    }

    None
}

/// Locate the TIFF header magic bytes within a byte slice.
fn find_tiff_header(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 4 {
        return None;
    }
    for i in 0..=bytes.len() - 4 {
        if &bytes[i..i + 4] == b"II\x2A\x00" || &bytes[i..i + 4] == b"MM\x00\x2A" {
            return Some(i);
        }
    }
    None
}

/// Complete HEIC metadata removal without breaking container structural boxes (pitm, iloc, iinf).
pub fn strip_all_heic_metadata(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = bytes.to_vec();

    // 1. Zero out the EXIF TIFF payload bytes
    if let Some((start, len)) = find_heic_exif_payload_range(&out) {
        out[start..start + len].fill(0);
    }

    // 2. Replace 'Exif' item type tags in the 'meta' box bounds with 'free'
    if let Some((meta_start, meta_end)) = find_heic_meta_bounds(&out) {
        let meta_slice = &mut out[meta_start..meta_end];
        for i in 0..meta_slice.len().saturating_sub(4) {
            if &meta_slice[i..i + 4] == b"Exif" {
                meta_slice[i..i + 4].copy_from_slice(b"free");
            }
        }
    }

    Ok(out)
}

/// Helper to locate the exact offset and length of the EXIF payload inside HEIC bytes.
fn find_heic_exif_payload_range(bytes: &[u8]) -> Option<(usize, usize)> {
    let (meta_start, meta_end) = find_heic_meta_bounds(bytes)?;
    let meta_slice = &bytes[meta_start..meta_end];

    let tiff_offset = find_tiff_header(meta_slice)?;
    let abs_start = meta_start + tiff_offset;
    let payload_len = meta_end.saturating_sub(abs_start);

    Some((abs_start, payload_len))
}

fn strip_jpeg_app_segments(bytes: &[u8], should_remove: impl Fn(u8) -> bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    if bytes.len() < 2 {
        return bytes.to_vec();
    }
    out.extend_from_slice(&bytes[0..2]);
    i += 2;

    while i + 3 < bytes.len() {
        if bytes[i] != 0xFF {
            out.extend_from_slice(&bytes[i..]);
            break;
        }

        let marker = bytes[i + 1];
        let seg_start = i;

        if matches!(marker, 0xD0..=0xD9) {
            out.extend_from_slice(&bytes[i..i + 2]);
            i += 2;
            continue;
        }

        if i + 3 >= bytes.len() {
            out.extend_from_slice(&bytes[i..]);
            break;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        let seg_end = i + 2 + len;
        if seg_end > bytes.len() {
            out.extend_from_slice(&bytes[i..]);
            break;
        }

        if (0xE0..=0xEF).contains(&marker) && should_remove(marker) {
            // skip
        } else {
            out.extend_from_slice(&bytes[seg_start..seg_end]);
        }

        i = seg_end;
    }

    out
}

// ── PNG helpers ───────────────────────────────────────────────────────────────
// fn strip_png_metadata(png: &[u8]) -> Vec<u8> {
//     rewrite_png_chunks(png, |ctype, _| ctype != b"eXIf")
// }

fn rewrite_png_exif_without_gps(png: &[u8]) -> Result<Vec<u8>> {
    let mut out = png.to_vec();
    foreach_png_chunk_mut(&mut out, |ctype, data| {
        if ctype == b"eXIf" {
            if let Ok(new_tiff) = strip_gps_from_tiff(data) {
                data.copy_from_slice(&new_tiff);
            }
        }
    })?;
    Ok(out)
}

/// Walk PNG chunks, optionally skipping entire chunks.
fn rewrite_png_chunks(
    png: &[u8],
    keep_chunk: impl Fn(&[u8; 4], &[u8]) -> bool,
) -> Vec<u8> {
    if !is_png(png) {
        return png.to_vec();
    }

    let mut out = Vec::with_capacity(png.len());
    out.extend_from_slice(&PNG_SIG);

    let mut i = 8;

    while i + 12 <= png.len() {
        let len = u32::from_be_bytes([
            png[i],
            png[i + 1],
            png[i + 2],
            png[i + 3],
        ]) as usize;

        let chunk_end = match i.checked_add(12 + len) {
            Some(end) if end <= png.len() => end,
            _ => return png.to_vec(), // invalid PNG: preserve original
        };

        let ctype: [u8; 4] = match png[i + 4..i + 8].try_into() {
            Ok(v) => v,
            Err(_) => return png.to_vec(),
        };

        let data = &png[i + 8..i + 8 + len];

        if keep_chunk(&ctype, data) {
            out.extend_from_slice(&png[i..chunk_end]);
        }

        i = chunk_end;
    }

    // A valid PNG must end exactly after IEND.
    if i != png.len() {
        return png.to_vec();
    }

    out
}

fn foreach_png_chunk_mut(
    png: &mut [u8],
    mut f: impl FnMut(&[u8; 4], &mut [u8]),
) -> Result<()> {
    if !png.starts_with(&PNG_SIG) {
        return Ok(());
    }

    let mut i = 8;
    while i + 12 <= png.len() {
        let len = u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]) as usize;
        let ctype: [u8; 4] = png[i + 4..i + 8].try_into().unwrap();
        let data_start = i + 8;
        let data_end = data_start + len;
        let crc_start = data_end;
        let chunk_end = crc_start + 4;
        if chunk_end > png.len() {
            break;
        }

        f(&ctype, &mut png[data_start..data_end]);

        // Recompute CRC for type + data
        let mut crc_input = Vec::with_capacity(4 + len);
        crc_input.extend_from_slice(&ctype);
        crc_input.extend_from_slice(&png[data_start..data_end]);
        let crc = png_crc32(&crc_input);
        png[crc_start..chunk_end].copy_from_slice(&crc.to_be_bytes());

        i = chunk_end;
    }

    Ok(())
}

fn png_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

// ── WebP helpers ──────────────────────────────────────────────────────────────
// fn strip_webp_exif(webp: &[u8]) -> Vec<u8> {
//     rebuild_webp_chunks(webp, |fourcc, payload| {
//         if fourcc == b"EXIF" {
//             None
//         } else {
//             Some(payload.to_vec())
//         }
//     })
//     .unwrap_or_else(|| webp.to_vec())
// }

fn rewrite_webp_exif_without_gps(webp: &[u8]) -> Result<Vec<u8>> {
    rebuild_webp_chunks(webp, |fourcc, payload| {
        if fourcc == b"EXIF" {
            strip_gps_from_tiff(payload).ok()
        } else {
            Some(payload.to_vec())
        }
    })
    .ok_or_else(|| anyhow::anyhow!("WebP EXIF rewrite failed"))
}

fn rebuild_webp_chunks(
    webp: &[u8],
    transform: impl Fn(&[u8; 4], &[u8]) -> Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    if !is_webp(webp) || webp.len() < 12 {
        return None;
    }

    let mut out = Vec::new();
    out.extend_from_slice(&webp[0..4]); // RIFF
    out.extend_from_slice(&[0, 0, 0, 0]); // placeholder size
    out.extend_from_slice(&webp[8..12]); // WEBP

    let file_size = u32::from_le_bytes([webp[4], webp[5], webp[6], webp[7]]) as usize;
    let mut pos = 12;
    let end = (8 + file_size).min(webp.len());

    while pos + 8 <= end {
        let fourcc: [u8; 4] = webp[pos..pos + 4].try_into().ok()?;
        let size = u32::from_le_bytes([
            webp[pos + 4],
            webp[pos + 5],
            webp[pos + 6],
            webp[pos + 7],
        ]) as usize;
        let payload_start = pos + 8;
        let payload_end = payload_start + size;
        if payload_end > webp.len() {
            break;
        }

        if let Some(new_payload) = transform(&fourcc, &webp[payload_start..payload_end]) {
            out.extend_from_slice(&fourcc);
            out.extend_from_slice(&(new_payload.len() as u32).to_le_bytes());
            out.extend_from_slice(&new_payload);
            if new_payload.len() % 2 != 0 {
                out.push(0);
            }
        }

        pos = payload_end;
        if size % 2 != 0 {
            pos += 1;
        }
    }

    let riff_size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff_size.to_le_bytes());
    Some(out)
}

// ── TIFF helpers ──────────────────────────────────────────────────────────
fn extract_jpeg_exif_tiff(bytes: &[u8]) -> Option<&[u8]> {
    if !is_jpeg(bytes) {
        return None;
    }
    let mut i = 2;

    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        i += 2;

        // SOS / image data starts here
        if marker == 0xDA {
            break;
        }
        if i + 2 > bytes.len() {
            return None;
        }

        let len = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;

        if marker == 0xE1 && len >= 8 {
            let start = i + 2;

            if start + 6 <= bytes.len()
                && &bytes[start..start + 6] == b"Exif\0\0"
            {
                return Some(&bytes[start + 6..i + len]);
            }
        }
        i += len;
    }
    None
}

fn parse_orientation_from_ifd(tiff: &[u8]) -> Option<u32> {
    if tiff.len() < 8 {
        return None;
    }

    let little_endian = match &tiff[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };

    let read_u16 = |buf: &[u8], offset: usize| -> Option<u16> {
        buf.get(offset..offset + 2).map(|b| {
            if little_endian {
                u16::from_le_bytes([b[0], b[1]])
            } else {
                u16::from_be_bytes([b[0], b[1]])
            }
        })
    };

    let read_u32 = |buf: &[u8], offset: usize| -> Option<u32> {
        buf.get(offset..offset + 4).map(|b| {
            if little_endian {
                u32::from_le_bytes([b[0], b[1], b[2], b[3]])
            } else {
                u32::from_be_bytes([b[0], b[1], b[2], b[3]])
            }
        })
    };

    let ifd_offset = read_u32(tiff, 4)? as usize;
    let entry_count = read_u16(tiff, ifd_offset)? as usize;

    for e in 0..entry_count {
        let entry_offset = ifd_offset + 2 + e * 12;
        let tag = read_u16(tiff, entry_offset)?;
        if tag == 0x0112 {
            return Some(read_u16(tiff, entry_offset + 8)? as u32);
        }
    }

    None
}

fn rewrite_jpeg_exif_without_gps(jpeg: &[u8]) -> Result<Vec<u8>> {
    let mut i = 2_usize;

    while i + 3 < jpeg.len() {
        if jpeg[i] != 0xFF {
            break;
        }
        let marker = jpeg[i + 1];
        let len = u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]) as usize;
        let seg_end = i + 2 + len;

        if marker == 0xE1 {
            let payload = &jpeg[i + 4..seg_end];
            if payload.starts_with(EXIF_HEADER) {
                let tiff_data = &payload[EXIF_HEADER.len()..];
                let new_tiff = strip_gps_from_tiff(tiff_data)?;
                return rewrite_jpeg_exif_segment(jpeg, &new_tiff);
            }
        }

        i = seg_end;
    }

    Ok(jpeg.to_vec())
}

fn rewrite_jpeg_exif_segment(jpeg: &[u8], new_tiff: &[u8]) -> Result<Vec<u8>> {
    anyhow::ensure!(
        jpeg.len() >= 2 && jpeg.starts_with(&[0xFF, 0xD8]),
        "invalid JPEG"
    );

    let mut out = Vec::with_capacity(jpeg.len() + EXIF_HEADER.len() + new_tiff.len() + 4);
    out.extend_from_slice(&jpeg[..2]); // SOI
    let mut i = 2;
    let mut inserted = false;

    while i + 1 < jpeg.len() {
        // Every metadata segment begins with 0xFF.
        if jpeg[i] != 0xFF {
            break;
        }
        let marker = jpeg[i + 1];

        match marker {
            // APP0..APP15 and COM are the only segments we walk through looking
            // for EXIF. They all have a 2-byte length field.
            0xE0..=0xEF | 0xFE => {
                anyhow::ensure!(i + 4 <= jpeg.len(), "truncated JPEG segment");

                let len = u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]) as usize;
                anyhow::ensure!(len >= 2, "invalid JPEG segment length");

                let seg_end = i + 2 + len;
                anyhow::ensure!(seg_end <= jpeg.len(), "truncated JPEG segment");

                let payload = &jpeg[i + 4..seg_end];

                if marker == 0xE1 && payload.starts_with(EXIF_HEADER) {
                    // Replace existing EXIF.
                    write_jpeg_app1_exif_segment(&mut out, new_tiff);
                    inserted = true;
                } else {
                    out.extend_from_slice(&jpeg[i..seg_end]);
                }
                i = seg_end;
            }

            // First non-APP/COM marker: insert EXIF here (if needed) and then
            // copy the remainder of the JPEG unchanged.
            _ => {
                if !inserted {
                    write_jpeg_app1_exif_segment(&mut out, new_tiff);
                }
                out.extend_from_slice(&jpeg[i..]);
                return Ok(out);
            }
        }
    }
    // Degenerate JPEG consisting only of SOI + APPn/COM segments.
    if !inserted {
        write_jpeg_app1_exif_segment(&mut out, new_tiff);
    }

    if i < jpeg.len() {
        out.extend_from_slice(&jpeg[i..]);
    }
    Ok(out)
}

fn write_jpeg_app1_exif_segment(out: &mut Vec<u8>, tiff: &[u8]) {
    let payload_len = EXIF_HEADER.len() + tiff.len();
    let seg_len = (payload_len + 2) as u16;
    out.push(0xFF);
    out.push(0xE1);
    out.extend_from_slice(&seg_len.to_be_bytes());
    out.extend_from_slice(EXIF_HEADER);
    out.extend_from_slice(tiff);
}

fn inject_exif_into_jpeg(jpeg: &[u8], tiff: &[u8]) -> Result<Vec<u8>> {
    rewrite_jpeg_exif_segment(jpeg, tiff)
}

fn inject_exif_into_png(png: &[u8], tiff: &[u8]) -> Result<Vec<u8>> {
    if !is_png(png) {
        return Ok(png.to_vec());
    }

    let without_exif = rewrite_png_chunks(png, |ctype, _| ctype != b"eXIf");

    let mut rebuilt = Vec::with_capacity(without_exif.len() + 12 + tiff.len());
    rebuilt.extend_from_slice(&without_exif[0..8]);

    let mut i = 8;
    let mut inserted = false;
    while i + 12 <= without_exif.len() {
        let len = u32::from_be_bytes([
            without_exif[i],
            without_exif[i + 1],
            without_exif[i + 2],
            without_exif[i + 3],
        ]) as usize;
        let chunk_end = i + 12 + len;
        if chunk_end > without_exif.len() {
            rebuilt.extend_from_slice(&without_exif[i..]);
            break;
        }

        rebuilt.extend_from_slice(&without_exif[i..chunk_end]);

        let ctype: [u8; 4] = without_exif[i + 4..i + 8].try_into().unwrap();
        if !inserted && &ctype == b"IHDR" {
            append_png_exif_chunk(&mut rebuilt, tiff);
            inserted = true;
        }

        i = chunk_end;
    }

    if !inserted {
        return Ok(png.to_vec());
    }

    Ok(rebuilt)
}

fn append_png_exif_chunk(out: &mut Vec<u8>, tiff: &[u8]) {
    out.extend_from_slice(&(tiff.len() as u32).to_be_bytes());
    out.extend_from_slice(b"eXIf");
    out.extend_from_slice(tiff);
    let mut crc_input = Vec::with_capacity(4 + tiff.len());
    crc_input.extend_from_slice(b"eXIf");
    crc_input.extend_from_slice(tiff);
    out.extend_from_slice(&png_crc32(&crc_input).to_be_bytes());
}

fn inject_exif_into_webp(webp: &[u8], tiff: &[u8]) -> Result<Vec<u8>> {
    let out = rebuild_webp_chunks(webp, |fourcc, payload| {
        if fourcc == b"EXIF" {
            Some(tiff.to_vec())
        } else {
            Some(payload.to_vec())
        }
    })
    .ok_or_else(|| anyhow::anyhow!("WebP EXIF injection failed"))?;

    if out.windows(4).any(|w| w == b"EXIF") {
        return Ok(out);
    }

    let mut with_exif = Vec::with_capacity(out.len() + 8 + tiff.len());
    with_exif.extend_from_slice(&out[0..12]);
    with_exif.extend_from_slice(b"EXIF");
    with_exif.extend_from_slice(&(tiff.len() as u32).to_le_bytes());
    with_exif.extend_from_slice(tiff);
    if tiff.len() % 2 != 0 {
        with_exif.push(0);
    }
    with_exif.extend_from_slice(&out[12..]);
    let riff_size = (with_exif.len() - 8) as u32;
    with_exif[4..8].copy_from_slice(&riff_size.to_le_bytes());
    Ok(with_exif)
}

fn extract_png_exif_tiff(png: &[u8]) -> Option<Vec<u8>> {
    if !is_png(png) {
        return None;
    }

    let mut i = 8;
    while i + 12 <= png.len() {
        let len = u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]) as usize;
        let chunk_end = i + 12 + len;
        if chunk_end > png.len() {
            break;
        }
        let ctype: [u8; 4] = png[i + 4..i + 8].try_into().ok()?;
        if &ctype == b"eXIf" {
            return Some(png[i + 8..i + 8 + len].to_vec());
        }
        i = chunk_end;
    }
    None
}

fn extract_webp_exif_tiff(webp: &[u8]) -> Option<Vec<u8>> {
    if !is_webp(webp) {
        return None;
    }

    let file_size = u32::from_le_bytes([webp[4], webp[5], webp[6], webp[7]]) as usize;
    let mut pos = 12;
    let end = (8 + file_size).min(webp.len());

    while pos + 8 <= end {
        let fourcc: [u8; 4] = webp[pos..pos + 4].try_into().ok()?;
        let size = u32::from_le_bytes([webp[pos + 4], webp[pos + 5], webp[pos + 6], webp[pos + 7]])
            as usize;
        let payload_start = pos + 8;
        let payload_end = payload_start + size;
        if payload_end > webp.len() {
            break;
        }
        if &fourcc == b"EXIF" {
            return Some(webp[payload_start..payload_end].to_vec());
        }
        pos = payload_end;
        if size % 2 != 0 {
            pos += 1;
        }
    }
    None
}

fn reset_orientation_in_tiff(tiff: &mut [u8]) {
    if tiff.len() < 8 {
        return;
    }

    let little_endian = match &tiff[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return,
    };

    let read_u16 = |b: &[u8], o: usize| -> Option<u16> {
        b.get(o..o + 2).map(|s| {
            if little_endian {
                u16::from_le_bytes([s[0], s[1]])
            } else {
                u16::from_be_bytes([s[0], s[1]])
            }
        })
    };
    let read_u32 = |b: &[u8], o: usize| -> Option<u32> {
        b.get(o..o + 4).map(|s| {
            if little_endian {
                u32::from_le_bytes([s[0], s[1], s[2], s[3]])
            } else {
                u32::from_be_bytes([s[0], s[1], s[2], s[3]])
            }
        })
    };
    let write_u16 = |b: &mut [u8], o: usize, v: u16| {
        let bytes = if little_endian {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        };
        if o + 2 <= b.len() {
            b[o..o + 2].copy_from_slice(&bytes);
        }
    };

    let ifd_offset = match read_u32(tiff, 4) {
        Some(o) => o as usize,
        None => return,
    };
    let entry_count = match read_u16(tiff, ifd_offset) {
        Some(c) => c as usize,
        None => return,
    };

    for e in 0..entry_count {
        let entry_offset = ifd_offset + 2 + e * 12;
        if read_u16(tiff, entry_offset) == Some(0x0112) {
            write_u16(tiff, entry_offset + 8, 1);
            break;
        }
    }
}

pub fn strip_gps_from_tiff(tiff: &[u8]) -> Result<Vec<u8>> {
    let mut buf = tiff.to_vec();

    if buf.len() < 8 {
        return Ok(buf);
    }

    let little_endian = match &buf[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Ok(buf),
    };

    let read_u16 = |b: &[u8], o: usize| -> Option<u16> {
        b.get(o..o + 2).map(|s| {
            if little_endian {
                u16::from_le_bytes([s[0], s[1]])
            } else {
                u16::from_be_bytes([s[0], s[1]])
            }
        })
    };
    let read_u32 = |b: &[u8], o: usize| -> Option<u32> {
        b.get(o..o + 4).map(|s| {
            if little_endian {
                u32::from_le_bytes([s[0], s[1], s[2], s[3]])
            } else {
                u32::from_be_bytes([s[0], s[1], s[2], s[3]])
            }
        })
    };
    let write_u32 = |b: &mut Vec<u8>, o: usize, v: u32| {
        let bytes = if little_endian {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        };
        b[o..o + 4].copy_from_slice(&bytes);
    };

    let ifd_offset = match read_u32(&buf, 4) {
        Some(o) => o as usize,
        None => return Ok(buf),
    };

    let entry_count = match read_u16(&buf, ifd_offset) {
        Some(c) => c as usize,
        None => return Ok(buf),
    };

    for e in 0..entry_count {
        let entry_offset = ifd_offset + 2 + e * 12;
        if let Some(tag) = read_u16(&buf, entry_offset) {
            if tag == 0x8825 {
                write_u32(&mut buf, entry_offset + 8, 0);
                break;
            }
        }
    }

    Ok(buf)
}
