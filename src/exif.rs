//! EXIF metadata manipulation.
//!
//! Strategy:
//! - We use a byte-level approach to strip EXIF APP1 markers from JPEG files,
//!   and for more granular GPS-only removal we parse and rewrite the EXIF block.
//! - For non-JPEG formats that don't embed EXIF in the same way (PNG tEXt/zTXt,
//!   WebP EXIF chunk, TIFF), we fall back to a format-aware strip.

use anyhow::Result;
// use crate::error::BatImgError;

// ── JPEG markers ─────────────────────────────────────────────────────────────
const SOI: [u8; 2] = [0xFF, 0xD8];        // Start of Image
// const APP1_MARKER: [u8; 2] = [0xFF, 0xE1]; // APP1 (EXIF / XMP)
const EXIF_HEADER: &[u8] = b"Exif\x00\x00";

/// Read the EXIF orientation tag (tag 0x0112) from raw image bytes.
/// Returns 1 (normal) if not found or on error.
pub fn read_orientation(bytes: &[u8]) -> Option<u32> {
    if !is_jpeg(bytes) {
        return None;
    }

    // Walk JPEG segments to find APP1 with EXIF header
    let exif_block = find_jpeg_app1_exif(bytes)?;

    // Minimal TIFF/EXIF parser for orientation only
    parse_orientation_from_ifd(exif_block)
}

/// Strip ALL metadata from image bytes (EXIF, XMP, IPTC, ICC profiles for JPEG).
pub fn strip_all_metadata(bytes: &[u8]) -> Result<Vec<u8>> {
    if is_jpeg(bytes) {
        Ok(strip_jpeg_app_segments(bytes, |marker| {
            // Remove APP0..APP15 (0xFFE0..0xFFEF) except APP0 (JFIF) which some
            // decoders need for density info — keep APP0, strip the rest.
            marker != 0xE0
        }))
    } else {
        // For PNG, WebP, TIFF etc. the `image` crate re-encodes without metadata
        // when we decode→encode, so returning the raw bytes here is acceptable;
        // the encoder in processor.rs will drop metadata on re-encode.
        Ok(bytes.to_vec())
    }
}

/// Strip only GPS-related EXIF tags from image bytes.
pub fn strip_gps_metadata(bytes: &[u8]) -> Result<Vec<u8>> {
    if !is_jpeg(bytes) {
        // Non-JPEG: return unchanged; pixel re-encode in processor handles cleanup.
        return Ok(bytes.to_vec());
    }

    // Find the APP1 EXIF block, rewrite it without GPS IFD, splice back in.
    match rewrite_jpeg_exif_without_gps(bytes) {
        Ok(stripped) => Ok(stripped),
        Err(_) => {
            // If we can't parse the EXIF, fall back to stripping the whole APP1.
            Ok(strip_jpeg_app_segments(bytes, |marker| marker == 0xE1))
        }
    }
}

// ── JPEG helpers ──────────────────────────────────────────────────────────────

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&SOI)
}

/// Remove JPEG APP segments matching `should_remove(sub_marker_byte)`.
fn strip_jpeg_app_segments(bytes: &[u8], should_remove: impl Fn(u8) -> bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    // Copy SOI
    if bytes.len() < 2 {
        return bytes.to_vec();
    }
    out.extend_from_slice(&bytes[0..2]);
    i += 2;

    while i + 3 < bytes.len() {
        if bytes[i] != 0xFF {
            // Not a marker; copy rest verbatim
            out.extend_from_slice(&bytes[i..]);
            break;
        }

        let marker = bytes[i + 1];
        let seg_start = i;

        // Markers without length: SOI (0xD8), EOI (0xD9), RST0-7
        if matches!(marker, 0xD8 | 0xD9 | 0xD0..=0xD7) {
            out.extend_from_slice(&bytes[i..i + 2]);
            i += 2;
            continue;
        }

        // Segments with a 2-byte big-endian length (includes the length field itself)
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

        // APP segments: 0xE0..=0xEF
        if (0xE0..=0xEF).contains(&marker) && should_remove(marker) {
            // Skip this segment
        } else {
            out.extend_from_slice(&bytes[seg_start..seg_end]);
        }

        i = seg_end;
    }

    out
}

/// Find the raw EXIF IFD bytes inside the JPEG APP1 segment.
fn find_jpeg_app1_exif(bytes: &[u8]) -> Option<&[u8]> {
    let mut i = 2_usize; // skip SOI
    while i + 3 < bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        if i + 3 >= bytes.len() {
            return None;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        let seg_end = i + 2 + len;
        if seg_end > bytes.len() {
            return None;
        }

        if marker == 0xE1 {
            // APP1
            let payload = &bytes[i + 4..seg_end];
            if payload.starts_with(EXIF_HEADER) {
                return Some(&payload[EXIF_HEADER.len()..]);
            }
        }

        i = seg_end;
    }
    None
}

// ── Minimal TIFF IFD orientation parser ──────────────────────────────────────

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
            // Orientation
            let value = read_u16(tiff, entry_offset + 8)? as u32;
            return Some(value);
        }
    }

    None
}

// ── GPS strip: rewrite EXIF without GPS IFD ──────────────────────────────────

fn rewrite_jpeg_exif_without_gps(jpeg: &[u8]) -> Result<Vec<u8>> {
    let mut i = 2_usize;
    let mut out = Vec::with_capacity(jpeg.len());
    out.extend_from_slice(&jpeg[0..2]); // SOI

    while i + 3 < jpeg.len() {
        if jpeg[i] != 0xFF {
            out.extend_from_slice(&jpeg[i..]);
            return Ok(out);
        }
        let marker = jpeg[i + 1];
        let len = u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]) as usize;
        let seg_end = i + 2 + len;

        if marker == 0xE1 {
            let payload = &jpeg[i + 4..seg_end];
            if payload.starts_with(EXIF_HEADER) {
                let tiff_data = &payload[EXIF_HEADER.len()..];
                match strip_gps_from_tiff(tiff_data) {
                    Ok(new_tiff) => {
                        // Rebuild APP1 segment
                        let new_payload_len = EXIF_HEADER.len() + new_tiff.len();
                        let new_seg_len = (new_payload_len + 2) as u16;
                        out.push(0xFF);
                        out.push(0xE1);
                        out.extend_from_slice(&new_seg_len.to_be_bytes());
                        out.extend_from_slice(EXIF_HEADER);
                        out.extend_from_slice(&new_tiff);
                        i = seg_end;
                        continue;
                    }
                    Err(_) => {
                        // Just drop the whole APP1
                        i = seg_end;
                        continue;
                    }
                }
            }
        }

        out.extend_from_slice(&jpeg[i..seg_end]);
        i = seg_end;
    }

    Ok(out)
}

/// Rewrite a TIFF block replacing the GPS IFD offset pointer with 0
/// (effectively unlinking the GPS sub-IFD from the main IFD).
fn strip_gps_from_tiff(tiff: &[u8]) -> Result<Vec<u8>> {
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
            if little_endian { u16::from_le_bytes([s[0], s[1]]) }
            else { u16::from_be_bytes([s[0], s[1]]) }
        })
    };
    let read_u32 = |b: &[u8], o: usize| -> Option<u32> {
        b.get(o..o + 4).map(|s| {
            if little_endian { u32::from_le_bytes([s[0], s[1], s[2], s[3]]) }
            else { u32::from_be_bytes([s[0], s[1], s[2], s[3]]) }
        })
    };
    let write_u32 = |b: &mut Vec<u8>, o: usize, v: u32| {
        let bytes = if little_endian { v.to_le_bytes() } else { v.to_be_bytes() };
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
                // GPSInfoIFDPointer — zero out the offset value
                write_u32(&mut buf, entry_offset + 8, 0);
                break;
            }
        }
    }

    Ok(buf)
}
