//! High‑level metadata stripping and rewriting operations.

use crate::exif::container::{
    EXIF_HEADER, extract_exif_tiff, foreach_png_chunk_mut, is_jpeg, is_png, is_tiff, is_webp,
    png_crc32, rebuild_webp_chunks, rewrite_png_chunks,
};
use crate::exif::heic::{
    extract_heic_exif_raw, replace_heic_exif_payload, strip_all_heic_metadata,
};
use anyhow::{Context, Result};
use std::path::Path;

/// Fast byte‑level GPS stripping across all supported image formats.
pub fn strip_gps_metadata(bytes: &[u8]) -> Result<Vec<u8>> {
    if is_jpeg(bytes) {
        rewrite_jpeg_exif_without_gps(bytes)
    } else if is_png(bytes) {
        rewrite_png_exif_without_gps(bytes)
    } else if is_webp(bytes) {
        rewrite_webp_exif_without_gps(bytes)
    } else if is_tiff(bytes) {
        strip_gps_from_tiff(bytes)
    } else if crate::heic::is_heic_bytes(bytes) {
        rewrite_heic_exif_without_gps(bytes)
    } else {
        Ok(bytes.to_vec())
    }
}

/// Fast byte‑level complete metadata stripping across all supported image formats.
pub fn strip_all_metadata(bytes: &[u8]) -> Result<Vec<u8>> {
    if is_jpeg(bytes) {
        Ok(strip_jpeg_app_segments(bytes, |marker| marker != 0xE0))
    } else if is_png(bytes) {
        Ok(strip_png_metadata(bytes))
    } else if is_webp(bytes) {
        Ok(strip_webp_metadata(bytes))
    } else if is_tiff(bytes) {
        strip_gps_from_tiff(bytes)
    } else if crate::heic::is_heic_bytes(bytes) {
        strip_all_heic_metadata(bytes)
    } else {
        Ok(bytes.to_vec())
    }
}

/// Copy non‑GPS EXIF from a GPS‑stripped source image into freshly encoded output bytes.
pub fn rewrite_exif_metadata(output: &[u8], source_stripped: &[u8]) -> Result<Vec<u8>> {
    let Some(mut exif_tiff) = extract_exif_tiff(source_stripped) else {
        return Ok(output.to_vec());
    };
    reset_orientation_in_tiff(&mut exif_tiff);

    if is_jpeg(output) {
        rewrite_jpeg_exif_segment(output, &exif_tiff)
    } else if is_png(output) {
        inject_exif_into_png(output, &exif_tiff)
    } else if is_webp(output) {
        inject_exif_into_webp(output, &exif_tiff)
    } else if is_tiff(output) {
        Ok(exif_tiff)
    } else {
        Ok(output.to_vec())
    }
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

/// Graft GPS‑stripped EXIF from `source_stripped` into an on‑disk encoded output file.
pub fn write_exif_file(output_path: &Path, source_stripped: &[u8]) -> Result<()> {
    let encoded = std::fs::read(output_path)
        .with_context(|| format!("Cannot read {} for EXIF graft", output_path.display()))?;
    let grafted = rewrite_exif_metadata(&encoded, source_stripped)?;
    std::fs::write(output_path, grafted)
        .with_context(|| format!("Cannot write EXIF graft to {}", output_path.display()))?;
    Ok(())
}

/// Strip GPS from a raw TIFF/EXIF block by removing the GPSInfo IFD pointer tag.
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
    let write_u16 = |b: &mut Vec<u8>, o: usize, v: u16| {
        let bytes = if little_endian {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        };
        b[o..o + 2].copy_from_slice(&bytes);
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
        if let Some(tag) = read_u16(&buf, entry_offset)
            && tag == 0x8825
        {
            let next_entry = entry_offset + 12;
            let end_of_entries = ifd_offset + 2 + entry_count * 12;
            buf.copy_within(next_entry..end_of_entries + 4, entry_offset);
            let freed_space_start = end_of_entries + 4 - 12;
            let freed_space_end = end_of_entries + 4;
            if freed_space_end <= buf.len() {
                buf[freed_space_start..freed_space_end].fill(0);
            }
            write_u16(&mut buf, ifd_offset, (entry_count - 1) as u16);
            break;
        }
    }
    Ok(buf)
}

// ---- JPEG rewrite ---------------------------------------------------------

fn rewrite_jpeg_exif_without_gps(jpeg: &[u8]) -> Result<Vec<u8>> {
    let mut i = 2;
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
    out.extend_from_slice(&jpeg[..2]);
    let mut i = 2;
    let mut inserted = false;
    while i + 1 < jpeg.len() {
        if jpeg[i] != 0xFF {
            break;
        }
        let marker = jpeg[i + 1];
        match marker {
            0xE0..=0xEF | 0xFE => {
                anyhow::ensure!(i + 4 <= jpeg.len(), "truncated JPEG");
                let len = u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]) as usize;
                anyhow::ensure!(len >= 2, "invalid segment length");
                let seg_end = i + 2 + len;
                anyhow::ensure!(seg_end <= jpeg.len(), "truncated JPEG");
                let payload = &jpeg[i + 4..seg_end];
                if marker == 0xE1 && payload.starts_with(EXIF_HEADER) {
                    write_jpeg_app1_exif_segment(&mut out, new_tiff);
                    inserted = true;
                } else {
                    out.extend_from_slice(&jpeg[i..seg_end]);
                }
                i = seg_end;
            }
            _ => {
                if !inserted {
                    write_jpeg_app1_exif_segment(&mut out, new_tiff);
                }
                out.extend_from_slice(&jpeg[i..]);
                return Ok(out);
            }
        }
    }
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

// ---- PNG rewrite ----------------------------------------------------------

fn rewrite_png_exif_without_gps(png: &[u8]) -> Result<Vec<u8>> {
    let mut out = png.to_vec();
    foreach_png_chunk_mut(&mut out, |ctype, data| {
        if ctype == b"eXIf"
            && let Ok(new_tiff) = strip_gps_from_tiff(data)
        {
            data.copy_from_slice(&new_tiff);
        }
    })?;
    Ok(out)
}

fn strip_png_metadata(png: &[u8]) -> Vec<u8> {
    rewrite_png_chunks(png, |ctype, _| {
        !matches!(ctype, b"eXIf" | b"tEXt" | b"zTXt" | b"iTXt" | b"iCCP")
    })
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

// ---- WebP rewrite ---------------------------------------------------------

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

fn strip_webp_metadata(webp: &[u8]) -> Vec<u8> {
    rebuild_webp_chunks(webp, |fourcc, _payload| {
        if matches!(fourcc, b"EXIF" | b"XMP " | b"ICCP") {
            None
        } else {
            Some(vec![])
        }
    })
    .unwrap_or_else(|| webp.to_vec())
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
    // insert EXIF chunk after RIFF/WEBP header
    let mut with_exif = Vec::with_capacity(out.len() + 8 + tiff.len());
    with_exif.extend_from_slice(&out[0..12]);
    with_exif.extend_from_slice(b"EXIF");
    with_exif.extend_from_slice(&(tiff.len() as u32).to_le_bytes());
    with_exif.extend_from_slice(tiff);
    if !tiff.len().is_multiple_of(2) {
        with_exif.push(0);
    }
    with_exif.extend_from_slice(&out[12..]);
    let riff_size = (with_exif.len() - 8) as u32;
    with_exif[4..8].copy_from_slice(&riff_size.to_le_bytes());
    Ok(with_exif)
}

// ---- HEIC rewrite ---------------------------------------------------------

fn rewrite_heic_exif_without_gps(bytes: &[u8]) -> Result<Vec<u8>> {
    if let Some(exif_tiff) = extract_heic_exif_raw(bytes) {
        let stripped_tiff = strip_gps_from_tiff(&exif_tiff)?;
        return replace_heic_exif_payload(bytes, &stripped_tiff);
    }
    Ok(bytes.to_vec())
}
