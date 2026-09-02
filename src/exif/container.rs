//! Container detection and extraction of embedded TIFF/EXIF blocks.

// Format signatures
pub const SOI: [u8; 2] = [0xFF, 0xD8]; // JPEG Start of Image
pub const EXIF_HEADER: &[u8] = b"Exif\0\0"; // 6-byte byte array
pub const PNG_SIG: [u8; 8] = *b"\x89PNG\r\n\x1a\n";

pub fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&SOI)
}

pub fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&PNG_SIG)
}

pub fn is_tiff(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && matches!(&bytes[..4], b"II\x2A\x00" | b"MM\x00\x2A")
}

pub fn is_webp(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP"
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

// ---- JPEG -----------------------------------------------------------------

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
        if marker == 0xDA {
            break;
        }
        if i + 2 > bytes.len() {
            return None;
        }
        let len = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
        if marker == 0xE1 && len >= 8 {
            let start = i + 2;
            if start + 6 <= bytes.len() && &bytes[start..start + 6] == b"Exif\0\0" {
                return Some(&bytes[start + 6..i + len]);
            }
        }
        i += len;
    }
    None
}

// ---- PNG ------------------------------------------------------------------

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

/// Walk PNG chunks, optionally skipping entire chunks.
pub fn rewrite_png_chunks(png: &[u8], keep_chunk: impl Fn(&[u8; 4], &[u8]) -> bool) -> Vec<u8> {
    if !is_png(png) {
        return png.to_vec();
    }
    let mut out = Vec::with_capacity(png.len());
    out.extend_from_slice(&PNG_SIG);
    let mut i = 8;
    while i + 12 <= png.len() {
        let len = u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]) as usize;
        let chunk_end = match i.checked_add(12 + len) {
            Some(end) if end <= png.len() => end,
            _ => return png.to_vec(),
        };
        let ctype: [u8; 4] = png[i + 4..i + 8].try_into().unwrap_or([0; 4]);
        let data = &png[i + 8..i + 8 + len];
        if keep_chunk(&ctype, data) {
            out.extend_from_slice(&png[i..chunk_end]);
        }
        i = chunk_end;
    }
    if i != png.len() {
        return png.to_vec();
    }
    out
}

pub fn foreach_png_chunk_mut(
    png: &mut [u8],
    mut f: impl FnMut(&[u8; 4], &mut [u8]),
) -> Result<(), anyhow::Error> {
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
        // recompute CRC
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

// ---- WebP -----------------------------------------------------------------

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
        if !size.is_multiple_of(2) {
            pos += 1;
        }
    }
    None
}

/// Rebuild a WebP file, optionally transforming each chunk.
pub fn rebuild_webp_chunks(
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
        let size = u32::from_le_bytes([webp[pos + 4], webp[pos + 5], webp[pos + 6], webp[pos + 7]])
            as usize;
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
        if !size.is_multiple_of(2) {
            pos += 1;
        }
    }
    let riff_size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff_size.to_le_bytes());
    Some(out)
}
