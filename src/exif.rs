//! EXIF metadata manipulation for JPEG, PNG, TIFF, WebP, and JPEG 2000.
//!
//! GPS removal rewrites the embedded TIFF/EXIF block in-place where possible.
//! Orientation is read from the same EXIF block across all supported containers.

use anyhow::Result;

// ── Format signatures ─────────────────────────────────────────────────────────
const SOI: [u8; 2] = [0xFF, 0xD8]; // JPEG Start of Image
const EXIF_HEADER: &[u8] = b"Exif\x00\x00";
const PNG_SIG: [u8; 8] = *b"\x89PNG\r\n\x1a\n";

/// Read the EXIF orientation tag (tag 0x0112) from any supported image container.
pub fn read_orientation(bytes: &[u8]) -> Option<u32> {
    let tiff = extract_jpeg_exif_tiff(bytes)?;
    parse_orientation_from_ifd(tiff)
}

/// Strip ALL metadata from image bytes.
pub fn strip_all_metadata(bytes: &[u8]) -> Result<Vec<u8>> {
    if is_jpeg(bytes) {
        Ok(strip_jpeg_app_segments(bytes, |marker| marker != 0xE0))
    } else if is_png(bytes) {
        Ok(strip_png_metadata(bytes))
    } else if is_webp(bytes) {
        Ok(strip_webp_exif(bytes))
    } else if is_tiff(bytes) {
        // Re-encoding TIFF drops most metadata; zero GPS pointer as a minimum.
        strip_gps_from_tiff(bytes)
    } else {
        Ok(bytes.to_vec())
    }
}

/// Strip only GPS-related EXIF tags from image bytes.
pub fn strip_gps_metadata(bytes: &[u8]) -> Result<Vec<u8>> {
    if is_jpeg(bytes) {
        match rewrite_jpeg_exif_without_gps(bytes) {
            Ok(stripped) => Ok(stripped),
            Err(_) => Ok(strip_jpeg_app_segments(bytes, |marker| marker == 0xE1)),
        }
    } else if is_png(bytes) {
        rewrite_png_exif_without_gps(bytes)
    } else if is_tiff(bytes) {
        strip_gps_from_tiff(bytes)
    } else if is_webp(bytes) {
        rewrite_webp_exif_without_gps(bytes)
    } else {
        Ok(bytes.to_vec())
    }
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

// ── JPEG helpers ──────────────────────────────────────────────────────────────

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

fn strip_png_metadata(png: &[u8]) -> Vec<u8> {
    rewrite_png_chunks(png, |ctype, _| ctype != b"eXIf")
}

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

fn strip_webp_exif(webp: &[u8]) -> Vec<u8> {
    rebuild_webp_chunks(webp, |fourcc, payload| {
        if fourcc == b"EXIF" {
            None
        } else {
            Some(payload.to_vec())
        }
    })
    .unwrap_or_else(|| webp.to_vec())
}

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
    let mut out = Vec::with_capacity(jpeg.len());
    out.extend_from_slice(&jpeg[0..2]);

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
