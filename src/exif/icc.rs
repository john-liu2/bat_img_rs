//! ICC profile extraction and parsing.

use crate::exif::container::{
    extract_exif_tiff, is_png, is_tiff, is_webp, read_short_tag_from_tiff,
};
use exif_lib::{In, Reader, Tag};
use flate2::read::{DeflateDecoder, ZlibDecoder};
use std::io::Read;

/// Extract ICC profile name from container bytes.
pub fn get_icc_profile_name(format: &str, bytes: &[u8]) -> Option<String> {
    match format {
        "JPEG" | "JPG" => extract_icc_from_jpeg(bytes),
        "WEBP" => extract_icc_from_webp(bytes),
        "PNG" => extract_icc_from_png(bytes),
        "TIFF" => extract_icc_from_tiff(bytes),
        "HEIC" | "AVIF" => extract_icc_from_heic(bytes),
        _ => None,
    }
}

/// Determine a colour‑profile name from the EXIF `ColorSpace` tag (0xA001).
pub fn exif_color_profile_name(_format: &str, bytes: &[u8]) -> Option<String> {
    let tiff = extract_exif_tiff(bytes)?;
    let color_space = exif_color_space_tag(&tiff)?;
    match color_space {
        1 => Some("sRGB IEC61966-2.1".to_string()),
        2 => Some("Adobe RGB (1998)".to_string()),
        _ => None,
    }
}

// ---- parsing ICC profile ---------------------------------------------------

pub fn parse_icc_profile_name(icc: &[u8]) -> Option<String> {
    if icc.len() < 132 {
        return None;
    }
    let tag_count = u32::from_be_bytes([icc[128], icc[129], icc[130], icc[131]]) as usize;
    let mut pos = 132;
    let mut model_name = None;
    let mut mluc_name = None;
    let mut desc_name = None;

    for _ in 0..tag_count {
        if pos + 12 > icc.len() {
            break;
        }
        let sig = &icc[pos..pos + 4];
        let offset =
            u32::from_be_bytes([icc[pos + 4], icc[pos + 5], icc[pos + 6], icc[pos + 7]]) as usize;
        let size =
            u32::from_be_bytes([icc[pos + 8], icc[pos + 9], icc[pos + 10], icc[pos + 11]]) as usize;
        if offset + size <= icc.len() {
            let payload = &icc[offset..offset + size];
            if let Some(name) = parse_payload_text(payload) {
                if sig == b"dmdd" || sig == b"mmod" || sig == b"dscm" {
                    model_name = Some(name);
                } else if sig == b"mluc" {
                    mluc_name = Some(name);
                } else if sig == b"desc" {
                    desc_name = Some(name);
                }
            }
        }
        pos += 12;
    }
    model_name.or(mluc_name).or(desc_name)
}

fn parse_payload_text(payload: &[u8]) -> Option<String> {
    if payload.len() < 4 {
        return None;
    }
    let type_sig = &payload[0..4];
    if type_sig == b"mluc" && payload.len() >= 28 {
        let records =
            u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]) as usize;
        let rec_size =
            u32::from_be_bytes([payload[12], payload[13], payload[14], payload[15]]) as usize;
        if records > 0 && rec_size >= 12 {
            let str_len =
                u32::from_be_bytes([payload[20], payload[21], payload[22], payload[23]]) as usize;
            let str_off =
                u32::from_be_bytes([payload[24], payload[25], payload[26], payload[27]]) as usize;
            if str_off + str_len <= payload.len() {
                let utf16_bytes = &payload[str_off..str_off + str_len];
                let utf16: Vec<u16> = utf16_bytes
                    .chunks_exact(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                let s = String::from_utf16_lossy(&utf16)
                    .trim_matches('\0')
                    .trim()
                    .to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
    } else if type_sig == b"desc" && payload.len() >= 12 {
        let str_len =
            u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]) as usize;
        if 12 + str_len <= payload.len() {
            let end = (12 + str_len).min(payload.len());
            let s = String::from_utf8_lossy(&payload[12..end])
                .trim_matches('\0')
                .trim()
                .to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    } else if type_sig == b"text" && payload.len() > 8 {
        let s = String::from_utf8_lossy(&payload[8..])
            .trim_matches('\0')
            .trim()
            .to_string();
        if !s.is_empty() {
            return Some(s);
        }
    }
    None
}

// ---- format‑specific extractors -------------------------------------------

fn extract_icc_from_jpeg(bytes: &[u8]) -> Option<String> {
    let mut i = 2;
    let mut icc_data = Vec::new();
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            break;
        }
        let marker = bytes[i + 1];
        if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if marker == 0xE2 && len >= 14 {
            let payload = &bytes[i + 4..i + 2 + len];
            if payload.starts_with(b"ICC_PROFILE\0") {
                icc_data.extend_from_slice(&payload[14..]);
            }
        }
        i += 2 + len;
    }
    if !icc_data.is_empty() {
        parse_icc_profile_name(&icc_data)
    } else {
        None
    }
}

fn extract_icc_from_webp(bytes: &[u8]) -> Option<String> {
    if !is_webp(bytes) {
        return None;
    }
    let file_size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let end = (8 + file_size).min(bytes.len());
    let mut pos = 12;
    while pos + 8 <= end {
        let fourcc = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        if fourcc == b"ICCP" && pos + 8 + size <= bytes.len() {
            return parse_icc_profile_name(&bytes[pos + 8..pos + 8 + size]);
        }
        pos += 8 + size;
        if !size.is_multiple_of(2) {
            pos += 1;
        }
    }
    None
}

fn extract_icc_from_png(bytes: &[u8]) -> Option<String> {
    if !is_png(bytes) {
        return None;
    }
    let mut i = 8;
    while i + 12 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        let ctype = &bytes[i + 4..i + 8];
        if ctype == b"iCCP" {
            let data = &bytes[i + 8..i + 8 + len];
            if let Some(null_pos) = data.iter().position(|&b| b == 0) {
                let profile_name = String::from_utf8_lossy(&data[..null_pos]).into_owned();

                // Check bounds before slicing to avoid panics on malformed chunks
                if null_pos + 2 > data.len() {
                    return Some(profile_name);
                }
                // Skip the 1-byte null separator and the 1-byte compression method
                let compressed = &data[null_pos + 2..];

                let mut decompressed = Vec::new();
                let mut ok = false;

                // Try zlib decompression
                let mut decoder = ZlibDecoder::new(compressed);
                if decoder.read_to_end(&mut decompressed).is_ok() {
                    ok = true;
                }

                // If zlib fails, try raw deflate
                if !ok {
                    decompressed.clear();
                    let mut decoder = DeflateDecoder::new(compressed);
                    if decoder.read_to_end(&mut decompressed).is_ok() {
                        ok = true;
                    }
                }

                if ok {
                    // Try standard parser
                    if let Some(desc) = parse_icc_profile_name(&decompressed) {
                        return Some(desc);
                    }
                }
                // If all fails, return the chunk name as a last resort
                return Some(profile_name);
            }
        }
        i += 12 + len;
    }
    None
}

fn extract_icc_from_tiff(bytes: &[u8]) -> Option<String> {
    if !is_tiff(bytes) {
        return None;
    }
    // read IFD0 and find tag 34675
    let is_little = match &bytes[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let read_u16 = |b: &[u8], o: usize| -> Option<u16> {
        b.get(o..o + 2).map(|s| {
            if is_little {
                u16::from_le_bytes([s[0], s[1]])
            } else {
                u16::from_be_bytes([s[0], s[1]])
            }
        })
    };
    let read_u32 = |b: &[u8], o: usize| -> Option<u32> {
        b.get(o..o + 4).map(|s| {
            if is_little {
                u32::from_le_bytes([s[0], s[1], s[2], s[3]])
            } else {
                u32::from_be_bytes([s[0], s[1], s[2], s[3]])
            }
        })
    };
    if read_u16(bytes, 2)? != 42 {
        return None;
    }
    let ifd_offset = read_u32(bytes, 4)? as usize;
    if ifd_offset + 2 > bytes.len() {
        return None;
    }
    let num_entries = read_u16(bytes, ifd_offset)? as usize;
    let mut pos = ifd_offset + 2;
    for _ in 0..num_entries {
        if pos + 12 > bytes.len() {
            break;
        }
        let tag = read_u16(bytes, pos)?;
        let count = read_u32(bytes, pos + 4)? as usize;
        if tag == 34675 {
            let value_offset = read_u32(bytes, pos + 8)? as usize;
            if value_offset + count <= bytes.len() {
                return parse_icc_profile_name(&bytes[value_offset..value_offset + count]);
            }
        }
        pos += 12;
    }
    None
}

fn extract_icc_from_heic(bytes: &[u8]) -> Option<String> {
    let mut pos = 0;
    while let Some(idx) = bytes[pos..].windows(4).position(|w| w == b"colr") {
        let i = pos + idx;
        if i >= 4 && i + 8 <= bytes.len() {
            let ctype = &bytes[i + 4..i + 8];
            if ctype == b"prof" || ctype == b"rICC" {
                let box_size =
                    u32::from_be_bytes([bytes[i - 4], bytes[i - 3], bytes[i - 2], bytes[i - 1]])
                        as usize;
                if box_size >= 12 && (i - 4) + box_size <= bytes.len() {
                    let icc_payload = &bytes[i + 8..(i - 4) + box_size];
                    if let Some(name) = parse_icc_profile_name(icc_payload) {
                        return Some(name);
                    }
                }
            }
        }
        pos = i + 4;
    }
    None
}

fn exif_color_space_tag(tiff: &[u8]) -> Option<u16> {
    let mut reader = Reader::new();
    reader.continue_on_error(true);
    if let Ok(exif_data) = reader.read_raw(tiff.to_vec())
        && let Some(field) = exif_data.get_field(Tag::ColorSpace, In::PRIMARY)
    {
        return field.value.get_uint(0).map(|v| v as u16);
    }
    // fallback manual scan
    read_short_tag_from_tiff(tiff, 0xA001)
}
