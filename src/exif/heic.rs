//! HEIC‑specific EXIF handling (box parsing, extraction, replacement).

use anyhow::Result;

/// Extract raw TIFF bytes from the 'Exif' box in a HEIC file.
pub fn extract_heic_exif_raw(bytes: &[u8]) -> Option<Vec<u8>> {
    if let Some((start, len)) = find_heic_exif_box(bytes) {
        return Some(bytes[start..start + len].to_vec());
    }
    // fallback: scan for TIFF header
    let tiff_offset = find_tiff_header(bytes)?;
    Some(bytes[tiff_offset..].to_vec())
}

/// Replace the TIFF payload inside the 'Exif' box, update box size, and zero padding.
pub fn replace_heic_exif_payload(bytes: &[u8], new_tiff: &[u8]) -> Result<Vec<u8>> {
    if let Some((payload_start, payload_len)) = find_heic_exif_box(bytes) {
        let mut out = bytes.to_vec();
        let new_len = new_tiff.len();
        if payload_start + new_len <= out.len() {
            out[payload_start..payload_start + new_len].copy_from_slice(new_tiff);
        }
        let end_of_box = payload_start + payload_len;
        if end_of_box <= out.len() {
            out[payload_start + new_len..end_of_box].fill(0);
        }
        // update box size
        let box_size_pos = payload_start - 8 - 4;
        let new_box_size = 8 + 4 + new_len;
        if box_size_pos + 4 <= out.len() {
            out[box_size_pos..box_size_pos + 4]
                .copy_from_slice(&(new_box_size as u32).to_be_bytes());
        }
        return Ok(out);
    }
    // fallback: raw offset replacement (no size update)
    let tiff_offset =
        find_tiff_header(bytes).ok_or_else(|| anyhow::anyhow!("No Exif or TIFF header found"))?;
    let mut out = bytes.to_vec();
    if tiff_offset + new_tiff.len() <= out.len() {
        out[tiff_offset..tiff_offset + new_tiff.len()].copy_from_slice(new_tiff);
    }
    Ok(out)
}

/// Return the TIFF payload from a libheif EXIF metadata block (handles `Exif\0\0` prefix).
pub fn tiff_from_heic_metadata(data: &[u8]) -> Option<&[u8]> {
    if data.starts_with(b"Exif\0\0") {
        return data.get(6..).filter(|tiff| is_tiff_header(tiff));
    }
    if is_tiff_header(data) {
        return Some(data);
    }
    let offset = u32::from_be_bytes(data.get(..4)?.try_into().ok()?) as usize;
    let tiff_start = 4usize.checked_add(offset)?;
    data.get(tiff_start..).filter(|tiff| is_tiff_header(tiff))
}

/// Complete HEIC metadata removal: zero Exif box and rename it to "free".
pub fn strip_all_heic_metadata(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = bytes.to_vec();
    // zero all TIFF header magic bytes and following payload window
    while let Some(offset) = find_tiff_header(&out) {
        let end = (offset + 1024).min(out.len());
        out[offset..end].fill(0);
    }
    // rename any "Exif" boxes to "free" inside the meta box
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

// ---- private helpers -------------------------------------------------------

fn find_heic_exif_box(bytes: &[u8]) -> Option<(usize, usize)> {
    let (meta_start, meta_end) = find_heic_meta_bounds(bytes)?;
    let mut pos = meta_start + 4; // skip version/flags
    while pos + 8 <= meta_end {
        let box_size =
            u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
                as usize;
        let box_type = &bytes[pos + 4..pos + 8];
        let header_len = 8;
        if box_size < header_len || pos + box_size > meta_end {
            break;
        }
        if box_type == b"Exif" {
            let payload_start = pos + 8 + 4; // FullBox version/flags
            let payload_len = box_size - 8 - 4;
            return Some((payload_start, payload_len));
        }
        pos += box_size;
    }
    None
}

fn find_heic_meta_bounds(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut pos = 0;
    while pos + 8 <= bytes.len() {
        let mut box_size =
            u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
                as usize;
        let box_type = &bytes[pos + 4..pos + 8];
        let header_len = if box_size == 1 {
            if pos + 16 > bytes.len() {
                break;
            }
            box_size = u64::from_be_bytes([
                bytes[pos + 8],
                bytes[pos + 9],
                bytes[pos + 10],
                bytes[pos + 11],
                bytes[pos + 12],
                bytes[pos + 13],
                bytes[pos + 14],
                bytes[pos + 15],
            ]) as usize;
            16
        } else if box_size == 0 {
            bytes.len() - pos
        } else {
            8
        };
        if box_size < header_len
            || pos
                .checked_add(box_size)
                .is_none_or(|end| end > bytes.len())
        {
            break;
        }
        if box_type == b"meta" {
            return Some((pos + header_len, pos + box_size));
        }
        pos += box_size;
    }
    None
}

/// Robust scan for standard TIFF header signatures.
pub fn find_tiff_header(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 8 {
        return None;
    }
    // look for Exif\0\0 prefix
    for i in 0..bytes.len().saturating_sub(10) {
        if &bytes[i..i + 6] == b"Exif\0\0" {
            let start = i + 6;
            if bytes[start..start + 4] == [0x49, 0x49, 0x2A, 0x00]
                || bytes[start..start + 4] == [0x4D, 0x4D, 0x00, 0x2A]
            {
                return Some(start);
            }
        }
    }
    // scan for raw TIFF magic
    for i in 0..bytes.len().saturating_sub(4) {
        if bytes[i..i + 4] == [0x49, 0x49, 0x2A, 0x00]
            || bytes[i..i + 4] == [0x4D, 0x4D, 0x00, 0x2A]
        {
            return Some(i);
        }
    }
    None
}

fn is_tiff_header(bytes: &[u8]) -> bool {
    bytes.starts_with(b"II\x2A\x00") || bytes.starts_with(b"MM\x00\x2A")
}
