// exif/metadata.rs: High‑level metadata stripping and rewriting operations
// Copyright © 2026 - Present, John Liu

use crate::exif::container::{
    EXIF_HEADER, extract_exif_tiff, foreach_png_chunk_mut, is_jpeg, is_png, is_tiff, is_webp,
    png_crc32, rebuild_webp_chunks, rewrite_png_chunks,
};
use crate::exif::heic::{
    extract_heic_exif_raw, replace_heic_exif_payload, strip_all_heic_metadata,
};
use anyhow::{Context, Result};
use exif_lib::{Reader, experimental::Writer};
use std::io::Cursor;
use std::ops::RangeInclusive;
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
        strip_tiff_metadata(bytes)
    } else if crate::heic::is_heic_bytes(bytes) {
        strip_all_heic_metadata(bytes)
    } else {
        Ok(bytes.to_vec())
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
    let write_u32 = |b: &mut [u8], o: usize, v: u32| {
        let bytes = if little_endian {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        };
        if o + 4 <= b.len() {
            b[o..o + 4].copy_from_slice(&bytes);
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
            let typ = read_u16(tiff, entry_offset + 2).unwrap_or(3);
            let cnt = read_u32(tiff, entry_offset + 4).unwrap_or(1);
            let total_size = match typ {
                1 | 2 | 6 | 7 => cnt as usize,
                3 | 8 => cnt as usize * 2,
                4 | 9 | 11 => cnt as usize * 4,
                5 | 10 | 12 => cnt as usize * 8,
                _ => return,
            };
            if total_size <= 4 {
                if matches!(typ, 4 | 9 | 11) {
                    write_u32(tiff, entry_offset + 8, 1);
                } else {
                    write_u16(tiff, entry_offset + 8, 1);
                }
            } else if let Some(val_offset) = read_u32(tiff, entry_offset + 8) {
                let vo = val_offset as usize;
                if matches!(typ, 4 | 9 | 11) {
                    write_u32(tiff, vo, 1);
                } else {
                    write_u16(tiff, vo, 1);
                }
            }
            break;
        }
    }
}

/// Write GPS‑stripped EXIF from `source_stripped` into an encoded output file.
pub fn write_exif_file(
    output_path: &Path,
    source_stripped: &[u8],
    is_grayscale: bool,
) -> Result<()> {
    let encoded = std::fs::read(output_path)
        .with_context(|| format!("Cannot read {} for EXIF graft", output_path.display()))?;
    let grafted = rewrite_exif_metadata(&encoded, source_stripped, is_grayscale)?;
    std::fs::write(output_path, grafted)
        .with_context(|| format!("Cannot write EXIF graft to {}", output_path.display()))?;
    Ok(())
}

pub fn rewrite_exif_metadata(
    output: &[u8],
    source_stripped: &[u8],
    is_grayscale: bool,
) -> Result<Vec<u8>> {
    // A raw TIFF is already the EXIF blob, but it may also contain full
    // image strips.  Extract only the metadata IFDs so we never append
    // megabytes of source pixel data onto the output TIFF (which can
    // confuse readers that walk the IFD chain).
    let mut exif_tiff = if is_tiff(source_stripped) {
        extract_tiff_metadata_only(source_stripped, is_grayscale)
    } else if let Some(t) = extract_exif_tiff(source_stripped) {
        t
    } else {
        return Ok(output.to_vec());
    };
    reset_orientation_in_tiff(&mut exif_tiff);

    if is_grayscale {
        set_exif_color_space_to_uncalibrated(&mut exif_tiff);
    }
    if is_jpeg(output) {
        rewrite_jpeg_exif_segment(output, &exif_tiff)
    } else if is_png(output) {
        inject_exif_into_png(output, &exif_tiff)
    } else if is_webp(output) {
        inject_exif_into_webp(output, &exif_tiff)
    } else if is_tiff(output) {
        inject_exif_into_tiff(output, &exif_tiff)
    } else {
        Ok(output.to_vec())
    }
}

/// IFD0 tags that describe how the *source* image's pixels are stored
/// (dimensions, bit depth, compression, strip/tile layout, sample format,
/// JPEG-in-TIFF and YCbCr parameters, sub-image pointers, ...).
///
/// The output TIFF was produced by our own encoder, so it already carries the
/// correct values for all of these.  Grafting the source's versions onto it
/// makes readers decode the new pixel data with the old image's layout.
const TIFF_STRUCTURE_TAGS: &[RangeInclusive<u16>] = &[
    0x00FE..=0x00FF, // NewSubfileType, SubfileType
    0x0100..=0x0103, // ImageWidth, ImageLength, BitsPerSample, Compression
    0x0106..=0x010A, // Photometric, Thresholding, CellWidth/Length, FillOrder
    0x0111..=0x0111, // StripOffsets
    0x0115..=0x0119, // SamplesPerPixel, RowsPerStrip, StripByteCounts, Min/MaxSampleValue
    0x011C..=0x011C, // PlanarConfiguration
    0x0120..=0x0125, // FreeOffsets/ByteCounts, GrayResponse*, T4/T6Options
    0x013D..=0x013D, // Predictor
    0x0140..=0x0148, // ColorMap, HalftoneHints, Tile*, fax line counts
    0x014A..=0x014A, // SubIFDs (point at sub-image pixel data)
    0x014C..=0x0150, // Ink*, DotRange
    0x0152..=0x0156, // ExtraSamples, SampleFormat, S{Min,Max}SampleValue, TransferRange
    0x0200..=0x0209, // (old-style) JPEG-in-TIFF tags
    0x0211..=0x0214, // YCbCr coefficients, subsampling, positioning, reference black/white
];

fn is_tiff_structure_tag(tag: u16) -> bool {
    TIFF_STRUCTURE_TAGS.iter().any(|range| range.contains(&tag))
}

/// IFD0 tags that describe the *source's colour model* (RGB ICC profile,
/// RGB white point / primaries / transfer function).  They are only valid for
/// an image with the same channel layout, so they must never be attached to a
/// single-channel (grayscale) output: viewers try to apply the three-channel
/// profile to one-channel data and render a garbled image.
const TIFF_COLOUR_DESCRIPTION_TAGS: &[u16] = &[
    0x012D, // TransferFunction
    0x013E, // WhitePoint
    0x013F, // PrimaryChromaticities
    0x8773, // ICC profile
];

fn is_tiff_colour_description_tag(tag: u16) -> bool {
    TIFF_COLOUR_DESCRIPTION_TAGS.contains(&tag)
}

/// Tags whose (inline) value is the offset of another IFD.
const TIFF_IFD_POINTER_TAGS: &[u16] = &[
    0x014A, // SubIFDs
    0x8769, // ExifOffset
    0x8825, // GPSInfo
    0xA005, // InteroperabilityOffset (lives in the Exif sub-IFD)
];

/// TIFF requires IFDs and out-of-line values to start on a word boundary.
fn pad_to_even(buf: &mut Vec<u8>) {
    if !buf.len().is_multiple_of(2) {
        buf.push(0);
    }
}

/// Build a self-contained metadata-only TIFF from `source`, dropping all
/// pixel-layout tags (`ImageWidth`, `StripOffsets`, `TileOffsets`, …) and the
/// strips they reference.  Sub-IFDs (`ExifOffset`, `GPSInfo`, `Interop`) are
/// copied recursively, so EXIF values survive.
///
/// When `is_grayscale` is set, the source's colour-model description (ICC
/// profile, white point, primaries, transfer function) is dropped as well
/// because it does not describe a single-channel image.
fn extract_tiff_metadata_only(source: &[u8], is_grayscale: bool) -> Vec<u8> {
    if !is_tiff(source) || source.len() < 8 {
        return source.to_vec();
    }
    let le = match &source[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return source.to_vec(),
    };
    let ifd0_src = read_u32_tiff(source, 4, le).unwrap_or(8) as usize;

    let mut out: Vec<u8> = Vec::with_capacity(256);
    out.extend_from_slice(&source[0..4]);
    out.extend_from_slice(&[0u8; 4]); // IFD0 offset placeholder

    match copy_tiff_ifd_metadata(source, ifd0_src, le, &mut out, 0, is_grayscale) {
        Some(new_ifd0) => {
            write_u32_tiff(&mut out, 4, new_ifd0 as u32, le);
            out
        }
        None => source.to_vec(),
    }
}

fn copy_tiff_ifd_metadata(
    src: &[u8],
    src_ifd_off: usize,
    le: bool,
    out: &mut Vec<u8>,
    depth: usize,
    is_grayscale: bool,
) -> Option<usize> {
    if depth > 16 {
        return None;
    }
    // Sub-IFDs we follow and re-home inside `out`.  SubIFDs (0x014A) is not
    // in this list: it is a pixel-layout tag and is dropped in IFD0.
    const SUB_IFD_POINTERS: &[u16] = &[0x8769, 0x8825, 0xA005];

    let count = read_u16_tiff(src, src_ifd_off, le)? as usize;

    // Collect entries we want to keep.
    let mut kept: Vec<[u8; 12]> = Vec::with_capacity(count);
    for i in 0..count {
        let eo = src_ifd_off + 2 + i * 12;
        let mut entry = [0u8; 12];
        entry.copy_from_slice(src.get(eo..eo + 12)?);
        let tag = read_u16_tiff(&entry, 0, le)?;
        // Image-layout / colour-model tags only exist in IFD0; the Exif, GPS
        // and Interop sub-IFDs use unrelated tag numbers.
        if depth == 0
            && (is_tiff_structure_tag(tag) || (is_grayscale && is_tiff_colour_description_tag(tag)))
        {
            continue;
        }
        if SUB_IFD_POINTERS.contains(&tag) {
            let typ = read_u16_tiff(&entry, 2, le).unwrap_or(0);
            let cnt = read_u32_tiff(&entry, 4, le).unwrap_or(0) as usize;
            if cnt * tiff_type_size(typ) <= 4 {
                // Drop pointers that do not lead to a readable IFD instead of
                // copying a dangling offset into the new file.
                let sub_off = read_u32_tiff(&entry, 8, le).unwrap_or(0) as usize;
                let sub_count = match read_u16_tiff(src, sub_off, le) {
                    Some(c) if sub_off >= 8 => c as usize,
                    _ => continue,
                };
                if sub_off + 2 + sub_count * 12 > src.len() {
                    continue;
                }
            }
        }
        kept.push(entry);
    }

    // Reserve room for the IFD inside the output.
    pad_to_even(out);
    let new_ifd_off = out.len();
    let ifd_size = 2 + kept.len() * 12 + 4;
    out.resize(new_ifd_off + ifd_size, 0);
    write_u16_tiff(out, new_ifd_off, kept.len() as u16, le);

    for (i, entry) in kept.iter().enumerate() {
        let eo = new_ifd_off + 2 + i * 12;
        out[eo..eo + 8].copy_from_slice(&entry[0..8]);

        let tag = read_u16_tiff(entry, 0, le).unwrap_or(0);
        let typ = read_u16_tiff(entry, 2, le).unwrap_or(0);
        let cnt = read_u32_tiff(entry, 4, le).unwrap_or(0);
        let total = cnt as usize * tiff_type_size(typ);

        if SUB_IFD_POINTERS.contains(&tag) && total <= 4 {
            let sub_off = read_u32_tiff(entry, 8, le).unwrap_or(0) as usize;
            if let Some(new_sub) =
                copy_tiff_ifd_metadata(src, sub_off, le, out, depth + 1, is_grayscale)
            {
                write_u32_tiff(out, eo + 8, new_sub as u32, le);
            } else {
                out[eo + 8..eo + 12].copy_from_slice(&entry[8..12]);
            }
        } else if total > 4 {
            // External value — copy the referenced bytes.
            let val_off = read_u32_tiff(entry, 8, le).unwrap_or(0) as usize;
            let value_bytes = if val_off + total <= src.len() {
                src[val_off..val_off + total].to_vec()
            } else {
                vec![0u8; total]
            };
            pad_to_even(out);
            let new_off = out.len() as u32;
            out.extend_from_slice(&value_bytes);
            write_u32_tiff(out, eo + 8, new_off, le);
        } else {
            // Inline value — copy verbatim.
            out[eo + 8..eo + 12].copy_from_slice(&entry[8..12]);
        }
    }
    // Next IFD pointer already zeroed by resize().
    Some(new_ifd_off)
}

fn tiff_type_size(typ: u16) -> usize {
    match typ {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 => 4,
        5 | 10 | 12 => 8,
        _ => 0,
    }
}

fn read_u16_tiff(b: &[u8], o: usize, le: bool) -> Option<u16> {
    b.get(o..o + 2).map(|s| {
        if le {
            u16::from_le_bytes([s[0], s[1]])
        } else {
            u16::from_be_bytes([s[0], s[1]])
        }
    })
}
fn read_u32_tiff(b: &[u8], o: usize, le: bool) -> Option<u32> {
    b.get(o..o + 4).map(|s| {
        if le {
            u32::from_le_bytes([s[0], s[1], s[2], s[3]])
        } else {
            u32::from_be_bytes([s[0], s[1], s[2], s[3]])
        }
    })
}
fn write_u16_tiff(b: &mut [u8], o: usize, v: u16, le: bool) {
    let bytes = if le { v.to_le_bytes() } else { v.to_be_bytes() };
    if o + 2 <= b.len() {
        b[o..o + 2].copy_from_slice(&bytes);
    }
}
fn write_u32_tiff(b: &mut [u8], o: usize, v: u32, le: bool) {
    let bytes = if le { v.to_le_bytes() } else { v.to_be_bytes() };
    if o + 4 <= b.len() {
        b[o..o + 4].copy_from_slice(&bytes);
    }
}

/// Grafts EXIF and metadata tags from the source TIFF into an encoded TIFF
pub fn inject_exif_into_tiff(output: &[u8], exif_tiff: &[u8]) -> Result<Vec<u8>> {
    let mut result = output.to_vec();
    if result.len() < 8 {
        return Ok(result);
    }
    let le = match &result[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Ok(result),
    };

    let exif_le = match &exif_tiff[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Ok(result),
    };
    let exif_tiff = if le != exif_le {
        reencode_exif_tiff(exif_tiff, le)?
    } else {
        exif_tiff.to_vec()
    };

    let read_u16 = |b: &[u8], o: usize| -> Option<u16> {
        b.get(o..o + 2).map(|s| {
            if le {
                u16::from_le_bytes([s[0], s[1]])
            } else {
                u16::from_be_bytes([s[0], s[1]])
            }
        })
    };
    let read_u32 = |b: &[u8], o: usize| -> Option<u32> {
        b.get(o..o + 4).map(|s| {
            if le {
                u32::from_le_bytes([s[0], s[1], s[2], s[3]])
            } else {
                u32::from_be_bytes([s[0], s[1], s[2], s[3]])
            }
        })
    };
    let write_u16 = |b: &mut [u8], o: usize, v: u16| {
        if let Some(slice) = b.get_mut(o..o + 2) {
            slice.copy_from_slice(&(if le { v.to_le_bytes() } else { v.to_be_bytes() }));
        }
    };
    let write_u32 = |b: &mut [u8], o: usize, v: u32| {
        if let Some(slice) = b.get_mut(o..o + 4) {
            slice.copy_from_slice(&(if le { v.to_le_bytes() } else { v.to_be_bytes() }));
        }
    };

    let old_ifd0_offset = match read_u32(&result, 4) {
        Some(o) => o as usize,
        None => return Ok(result),
    };
    let old_count = match read_u16(&result, old_ifd0_offset) {
        Some(c) => c as usize,
        None => return Ok(result),
    };

    let mut output_tags = Vec::new();
    let mut entries = Vec::new();
    for i in 0..old_count {
        let entry_start = old_ifd0_offset + 2 + i * 12;
        if let Some(slice) = result.get(entry_start..entry_start + 12) {
            let tag = read_u16(slice, 0).unwrap_or(0);
            output_tags.push(tag);
            entries.push(slice.to_vec());
        }
    }
    let next_ifd = read_u32(&result, old_ifd0_offset + 2 + old_count * 12).unwrap_or(0);

    // Tags that must be taken from the source even if the destination already
    // has them (these were canonicalized earlier).
    const OVERRIDE_TAGS: &[u16] = &[
        0x0112, // Orientation
        0x8769, // ExifOffset (grayscale ColorSpace lives in the sub-IFD)
    ];

    // Pull over any tags from the source that aren't already in the destination IFD0
    let mut tags_to_add = Vec::new();
    if let Some(exif_ifd0) = read_u32(&exif_tiff, 4)
        && let Some(count) = read_u16(&exif_tiff, exif_ifd0 as usize)
    {
        for i in 0..count {
            let entry_start = exif_ifd0 as usize + 2 + i as usize * 12;
            if let Some(slice) = exif_tiff.get(entry_start..entry_start + 12) {
                let tag = read_u16(slice, 0).unwrap_or(0);
                if OVERRIDE_TAGS.contains(&tag) || !output_tags.contains(&tag) {
                    tags_to_add.push(slice.to_vec());
                }
            }
        }
    }

    if tags_to_add.is_empty() {
        return Ok(result);
    }

    // Drop destination entries whose tags will be replaced by the source.
    let add_tags: Vec<u16> = tags_to_add
        .iter()
        .map(|e| read_u16(e, 0).unwrap_or(0))
        .collect();
    entries.retain(|e| {
        let tag = read_u16(e, 0).unwrap_or(0);
        !add_tags.contains(&tag)
    });

    // TIFF requires IFDs to start on a word boundary.  An encoder can leave the
    // file with an odd length, and strict readers reject (or misparse) an IFD
    // at an odd offset.
    pad_to_even(&mut result);

    let final_entry_count = entries.len() + tags_to_add.len();
    let new_ifd0_size = 2 + final_entry_count * 12 + 4;
    let delta = (result.len() + new_ifd0_size) as u32;

    for mut entry in tags_to_add {
        let typ = read_u16(&entry, 2).unwrap_or(0);
        let cnt = read_u32(&entry, 4).unwrap_or(0);
        let size = match typ {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            5 | 10 | 12 => 8,
            _ => 0,
        };

        let total_size = cnt.saturating_mul(size);
        let tag = read_u16(&entry, 0).unwrap_or(0);

        if (total_size > 4 || TIFF_IFD_POINTER_TAGS.contains(&tag))
            && let Some(val_offset) = read_u32(&entry, 8)
        {
            write_u32(&mut entry, 8, val_offset.saturating_add(delta));
        }
        entries.push(entry);
    }

    entries.sort_by_key(|e| read_u16(e, 0).unwrap_or(0));

    let new_ifd0_offset = result.len() as u32;
    write_u32(&mut result, 4, new_ifd0_offset);

    let mut count_buf = vec![0; 2];
    write_u16(&mut count_buf, 0, entries.len() as u16);
    result.extend_from_slice(&count_buf);

    for e in entries {
        result.extend_from_slice(&e);
    }

    let mut next_ifd_buf = vec![0; 4];
    write_u32(&mut next_ifd_buf, 0, next_ifd);
    result.extend_from_slice(&next_ifd_buf);

    // Apply delta shifts uniformly throughout the appended EXIF source
    let mut shifted_exif = exif_tiff.to_vec();
    let mut visited = Vec::new();

    if let Some(ifd0) = read_u32(&shifted_exif, 4) {
        let mut to_visit = vec![ifd0];
        while let Some(ifd) = to_visit.pop() {
            if ifd == 0 || visited.contains(&ifd) {
                continue;
            }
            visited.push(ifd);

            let offset = ifd as usize;
            if let Some(count) = read_u16(&shifted_exif, offset) {
                for i in 0..(count as usize) {
                    let entry = offset + 2 + i * 12;
                    let tag = read_u16(&shifted_exif, entry).unwrap_or(0);
                    let typ = read_u16(&shifted_exif, entry + 2).unwrap_or(0);
                    let cnt = read_u32(&shifted_exif, entry + 4).unwrap_or(0);

                    let size = match typ {
                        1 | 2 | 6 | 7 => 1,
                        3 | 8 => 2,
                        4 | 9 | 11 => 4,
                        5 | 10 | 12 => 8,
                        _ => 0,
                    };
                    let total = cnt.saturating_mul(size);

                    if total > 4
                        && let Some(val_offset) = read_u32(&shifted_exif, entry + 8)
                    {
                        write_u32(
                            &mut shifted_exif,
                            entry + 8,
                            val_offset.saturating_add(delta),
                        );
                    }

                    // ExifOffset, GPSInfo, SubIFDs and the Interoperability
                    // pointer inside the Exif sub-IFD all hold IFD offsets that
                    // must move together with the appended blob.  A pointer
                    // that is left behind ends up inside the image's pixel data.
                    if TIFF_IFD_POINTER_TAGS.contains(&tag) {
                        if total <= 4 {
                            if let Some(sub_ifd) = read_u32(&shifted_exif, entry + 8) {
                                to_visit.push(sub_ifd);
                                write_u32(
                                    &mut shifted_exif,
                                    entry + 8,
                                    sub_ifd.saturating_add(delta),
                                );
                            }
                        } else {
                            if let Some(shifted_val) = read_u32(&shifted_exif, entry + 8) {
                                let orig = shifted_val.saturating_sub(delta);
                                for j in 0..(cnt as usize) {
                                    if let Some(sub) =
                                        read_u32(&shifted_exif, orig as usize + j * 4)
                                    {
                                        to_visit.push(sub);
                                        write_u32(
                                            &mut shifted_exif,
                                            orig as usize + j * 4,
                                            sub + delta,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }

                let next_ptr = offset + 2 + (count as usize) * 12;
                if let Some(next_ifd) = read_u32(&shifted_exif, next_ptr)
                    && next_ifd > 0
                {
                    to_visit.push(next_ifd);
                    write_u32(&mut shifted_exif, next_ptr, next_ifd + delta);
                }
            }
        }
    }

    result.extend_from_slice(&shifted_exif);
    Ok(result)
}

/// Re-serialize a TIFF EXIF block to the requested byte order.
fn reencode_exif_tiff(tiff: &[u8], little_endian: bool) -> Result<Vec<u8>> {
    let mut reader = Reader::new();
    reader.continue_on_error(true);
    let exif = reader
        .read_raw(tiff.to_vec())
        .or_else(|error| error.distill_partial_result(|_| {}))
        .with_context(|| "Failed to parse EXIF TIFF for endianness conversion")?;

    let mut writer = Writer::new();
    for field in exif.fields() {
        writer.push_field(field);
    }

    let mut buf = Cursor::new(Vec::new());
    writer
        .write(&mut buf, little_endian)
        .with_context(|| "Failed to re-encode EXIF TIFF")?;
    Ok(buf.into_inner())
}

/// Sets the EXIF ColorSpace tag (0xA001) in the Exif sub-IFD to Uncalibrated (0xFFFF).
fn set_exif_color_space_to_uncalibrated(tiff: &mut [u8]) {
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
    let write_u32 = |b: &mut [u8], o: usize, v: u32| {
        let bytes = if little_endian {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        };
        if o + 4 <= b.len() {
            b[o..o + 4].copy_from_slice(&bytes);
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

    // Locate the ExifOffset tag (0x8769) to jump into the sub-IFD
    let mut exif_ifd_offset = None;
    for e in 0..entry_count {
        let entry_offset = ifd_offset + 2 + e * 12;
        if read_u16(tiff, entry_offset) == Some(0x8769) {
            exif_ifd_offset = read_u32(tiff, entry_offset + 8).map(|v| v as usize);
            break;
        }
    }

    if let Some(exif_offset) = exif_ifd_offset
        && let Some(exif_entry_count) = read_u16(tiff, exif_offset)
    {
        for e in 0..(exif_entry_count as usize) {
            let entry_offset = exif_offset + 2 + e * 12;
            if read_u16(tiff, entry_offset) == Some(0xA001) {
                let typ = read_u16(tiff, entry_offset + 2).unwrap_or(3);
                let cnt = read_u32(tiff, entry_offset + 4).unwrap_or(1);
                let total_size = match typ {
                    1 | 2 | 6 | 7 => cnt as usize,
                    3 | 8 => cnt as usize * 2,
                    4 | 9 | 11 => cnt as usize * 4,
                    5 | 10 | 12 => cnt as usize * 8,
                    _ => return,
                };
                if total_size <= 4 {
                    if matches!(typ, 4 | 9 | 11) {
                        write_u32(tiff, entry_offset + 8, 0xFFFF);
                    } else {
                        write_u16(tiff, entry_offset + 8, 0xFFFF);
                    }
                } else if let Some(val_offset) = read_u32(tiff, entry_offset + 8) {
                    let vo = val_offset as usize;
                    if matches!(typ, 4 | 9 | 11) {
                        write_u32(tiff, vo, 0xFFFF);
                    } else {
                        write_u16(tiff, vo, 0xFFFF);
                    }
                }
                break;
            }
        }
    }
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

/// Remove EXIF/descriptive metadata from a TIFF without rewriting pixel data.
///
/// TIFF pixel data is referenced by offsets from IFD entries.  Clearing the
/// metadata entries in place therefore avoids changing any pixel-data offsets
/// or compression details.  EXIF/GPS pointers are also cleared, making their
/// sub-IFDs unreachable to TIFF/EXIF readers.
pub fn strip_tiff_metadata(tiff: &[u8]) -> Result<Vec<u8>> {
    let mut out = tiff.to_vec();
    if out.len() < 8 || !is_tiff(&out) {
        return Ok(out);
    }

    let little_endian = &out[0..2] == b"II";

    fn read_u16(b: &[u8], offset: usize, le: bool) -> Option<u16> {
        b.get(offset..offset + 2).map(|s| {
            if le {
                u16::from_le_bytes([s[0], s[1]])
            } else {
                u16::from_be_bytes([s[0], s[1]])
            }
        })
    }
    fn read_u32(b: &[u8], offset: usize, le: bool) -> Option<u32> {
        b.get(offset..offset + 4).map(|s| {
            if le {
                u32::from_le_bytes([s[0], s[1], s[2], s[3]])
            } else {
                u32::from_be_bytes([s[0], s[1], s[2], s[3]])
            }
        })
    }

    // These tags are descriptive metadata, not the information required to
    // locate/decode the image pixels.  The EXIF/GPS pointers are included so
    // the metadata sub-IFDs become unreachable.
    const METADATA_TAGS: &[u16] = &[
        0x010D, // DocumentName
        0x010E, // ImageDescription
        0x010F, // Make
        0x0110, // Model
        0x0112, // Orientation
        0x011A, // XResolution
        0x011B, // YResolution
        0x011D, // PageName
        0x0128, // ResolutionUnit
        0x0131, // Software
        0x0132, // DateTime
        0x013B, // Artist
        0x013C, // HostComputer
        0x013E, // WhitePoint
        0x013F, // PrimaryChromaticities
        0x0156, // TransferRange
        0x02BC, // XMP
        0x8298, // Copyright
        0x8769, // ExifOffset
        0x8773, // ICCProfile
        0x8825, // GPSInfo
    ];

    let first_ifd = match read_u32(&out, 4, little_endian) {
        Some(offset) if offset != 0 => offset as usize,
        _ => return Ok(out),
    };

    // Follow only the normal TIFF page/IFD chain.  Metadata sub-IFDs are
    // deliberately not followed because their parent pointers are removed.
    let mut pending = vec![first_ifd];
    let mut visited = Vec::new();

    while let Some(ifd) = pending.pop() {
        if visited.contains(&ifd) {
            continue;
        }

        let count = match read_u16(&out, ifd, little_endian) {
            Some(count) => count as usize,
            None => continue,
        };
        let entries_end = match ifd.checked_add(2 + count * 12 + 4) {
            Some(end) if end <= out.len() => end,
            _ => continue,
        };

        visited.push(ifd);

        for index in 0..count {
            let entry = ifd + 2 + index * 12;
            let tag = match read_u16(&out, entry, little_endian) {
                Some(tag) => tag,
                None => continue,
            };

            if METADATA_TAGS.contains(&tag) {
                // Tag 0 is undefined/reserved, so readers ignore the whole
                // entry while the fixed IFD layout remains intact.
                out[entry..entry + 12].fill(0);
            }
        }
        let next_ifd = read_u32(&out, entries_end - 4, little_endian).unwrap_or(0);
        if next_ifd != 0 {
            pending.push(next_ifd as usize);
        }
    }

    Ok(out)
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
