// Container detection and extraction of embedded TIFF/EXIF blocks.
// Copyright © 2026 - Present, John Liu
//
// `extract_exif_tiff` is the single entry point: given the bytes of a JPEG,
// PNG, WebP, TIFF or HEIC file it returns the raw TIFF-structured EXIF block,
// ready for `parse_exif_bytes`.  Every walker below is bounds-checked and
// returns "nothing found" on truncated or malformed input instead of panicking.

use super::heic::extract_heic_exif_tiff;

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
        jpeg_segments(bytes)
            .find_map(|(marker, payload)| {
                if marker == 0xE1 {
                    payload.strip_prefix(EXIF_HEADER)
                } else {
                    None
                }
            })
            .map(<[u8]>::to_vec)
    } else if is_png(bytes) {
        png_chunks(bytes)
            .find(|chunk| &chunk.kind == b"eXIf")
            .map(|chunk| strip_exif_prefix(chunk.data).to_vec())
    } else if is_webp(bytes) {
        webp_chunks(bytes)
            .find(|(fourcc, _)| fourcc == b"EXIF")
            .map(|(_, payload)| strip_exif_prefix(payload).to_vec())
    } else if is_tiff(bytes) {
        Some(bytes.to_vec())
    } else if crate::heic::is_heic_bytes(bytes) {
        extract_heic_exif_tiff(bytes)
    } else {
        None
    }
}

/// Some writers put the JPEG-style `Exif\0\0` prefix in front of the TIFF data
/// of PNG `eXIf` and WebP `EXIF` chunks.  A TIFF header can never start with
/// those bytes, so dropping the prefix when present is always safe.
fn strip_exif_prefix(payload: &[u8]) -> &[u8] {
    payload.strip_prefix(EXIF_HEADER).unwrap_or(payload)
}

// ---- TIFF -----------------------------------------------------------------

fn read_u16(b: &[u8], offset: usize, le: bool) -> Option<u16> {
    let s: [u8; 2] = b.get(offset..offset.checked_add(2)?)?.try_into().ok()?;
    Some(if le {
        u16::from_le_bytes(s)
    } else {
        u16::from_be_bytes(s)
    })
}

fn read_u32(b: &[u8], offset: usize, le: bool) -> Option<u32> {
    let s: [u8; 4] = b.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
    Some(if le {
        u32::from_le_bytes(s)
    } else {
        u32::from_be_bytes(s)
    })
}

/// Read the inline SHORT value of `target_tag` from IFD0 of a TIFF block.
pub fn read_short_tag_from_tiff(tiff: &[u8], target_tag: u16) -> Option<u16> {
    let le = match tiff.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let ifd = read_u32(tiff, 4, le)? as usize;
    let count = read_u16(tiff, ifd, le)? as usize;
    (0..count)
        .map(|i| ifd + 2 + i * 12)
        .find(|&entry| read_u16(tiff, entry, le) == Some(target_tag))
        .and_then(|entry| read_u16(tiff, entry + 8, le))
}

fn tiff_type_size(typ: u16) -> usize {
    match typ {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 => 8,
        _ => 0,
    }
}

/// Length in bytes of the TIFF structure that starts at `tiff[0]`: the header,
/// every IFD reachable from it (next-IFD links, Exif/GPS/Interop sub-IFDs) and
/// every out-of-line value or embedded thumbnail those IFDs reference.
///
/// TIFF has no length field, so this is how an EXIF block embedded in a larger
/// file (a HEIC `mdat`, say) is delimited exactly.  Unreadable pointers are
/// ignored, and the result never exceeds `tiff.len()`.
pub(crate) fn tiff_extent(tiff: &[u8]) -> Option<usize> {
    // Enough for any real file; stops crafted files from making us do endless work.
    const MAX_IFDS: usize = 64;

    if !is_tiff(tiff) {
        return None;
    }
    let le = tiff[0] == b'I';
    let mut end = 8usize;
    let mut todo = vec![read_u32(tiff, 4, le)? as usize];
    let mut seen: Vec<usize> = Vec::new();

    while let Some(ifd) = todo.pop() {
        // Offsets below 8 point into the header itself.
        if ifd < 8 || seen.contains(&ifd) || seen.len() >= MAX_IFDS {
            continue;
        }
        seen.push(ifd);
        let Some(count) = read_u16(tiff, ifd, le) else {
            continue;
        };
        let count = count as usize;
        end = end.max(ifd + 2 + count * 12 + 4);

        let (mut thumb_offset, mut thumb_len) = (0usize, 0usize);
        for i in 0..count {
            let e = ifd + 2 + i * 12;
            let (Some(tag), Some(typ), Some(n), Some(value)) = (
                read_u16(tiff, e, le),
                read_u16(tiff, e + 2, le),
                read_u32(tiff, e + 4, le),
                read_u32(tiff, e + 8, le),
            ) else {
                break;
            };
            let (n, value) = (n as usize, value as usize);
            let size = n.saturating_mul(tiff_type_size(typ));
            if size > 4 {
                end = end.max(value.saturating_add(size)); // out-of-line value
            }
            match tag {
                0x8769 | 0x8825 | 0xA005 => todo.push(value), // Exif, GPS, Interop
                0x0201 => thumb_offset = value,               // JPEGInterchangeFormat
                0x0202 => thumb_len = value,                  // JPEGInterchangeFormatLength
                _ => {}
            }
        }
        if thumb_offset > 0 && thumb_len > 0 {
            end = end.max(thumb_offset.saturating_add(thumb_len));
        }
        if let Some(next) = read_u32(tiff, ifd + 2 + count * 12, le) {
            todo.push(next as usize);
        }
    }
    Some(end.min(tiff.len()))
}

// ---- JPEG -----------------------------------------------------------------

/// Marker segments of a JPEG up to the start of scan, as `(marker, payload)`.
/// The payload excludes the two length bytes.  Fill bytes and the standalone
/// markers (TEM, RSTn) are handled; a truncated segment ends the iteration.
fn jpeg_segments(bytes: &[u8]) -> impl Iterator<Item = (u8, &[u8])> {
    let mut pos = if is_jpeg(bytes) {
        SOI.len()
    } else {
        bytes.len()
    };
    std::iter::from_fn(move || {
        if *bytes.get(pos)? != 0xFF {
            return None;
        }
        while bytes.get(pos + 1) == Some(&0xFF) {
            pos += 1; // fill byte
        }
        let marker = *bytes.get(pos + 1)?;
        pos += 2;
        match marker {
            0xDA | 0xD9 => None,                                // SOS / EOI: no more headers
            0x01 | 0xD0..=0xD7 => Some((marker, &bytes[0..0])), // standalone, no length
            _ => {
                let len = u16::from_be_bytes([*bytes.get(pos)?, *bytes.get(pos + 1)?]) as usize;
                // `len` counts its own two bytes; anything below 2 is corrupt.
                let payload = bytes.get(pos + 2..pos + len)?;
                pos += len;
                Some((marker, payload))
            }
        }
    })
}

// ---- PNG ------------------------------------------------------------------

struct PngChunk<'a> {
    kind: [u8; 4],
    data: &'a [u8],
    /// Offset of the chunk's length field, and one past its CRC.
    start: usize,
    end: usize,
}

/// Well-formed PNG chunks, in order.  Iteration stops at the first chunk that
/// does not fit in the file.
fn png_chunks(png: &[u8]) -> impl Iterator<Item = PngChunk<'_>> {
    let mut pos = if is_png(png) {
        PNG_SIG.len()
    } else {
        png.len()
    };
    std::iter::from_fn(move || {
        let head = png.get(pos..pos.checked_add(8)?)?;
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let kind = [head[4], head[5], head[6], head[7]];
        let data_end = (pos + 8).checked_add(len)?;
        let end = data_end.checked_add(4)?; // CRC
        if end > png.len() {
            return None;
        }
        let chunk = PngChunk {
            kind,
            data: &png[pos + 8..data_end],
            start: pos,
            end,
        };
        pos = end;
        Some(chunk)
    })
}

/// Walk PNG chunks, optionally skipping entire chunks.  A damaged file
/// (truncated chunk, trailing bytes) is returned unchanged.
pub fn rewrite_png_chunks(png: &[u8], keep_chunk: impl Fn(&[u8; 4], &[u8]) -> bool) -> Vec<u8> {
    if !is_png(png) {
        return png.to_vec();
    }
    let mut out = Vec::with_capacity(png.len());
    out.extend_from_slice(&PNG_SIG);
    let mut consumed = PNG_SIG.len();
    for chunk in png_chunks(png) {
        if keep_chunk(&chunk.kind, chunk.data) {
            out.extend_from_slice(&png[chunk.start..chunk.end]);
        }
        consumed = chunk.end;
    }
    if consumed != png.len() {
        return png.to_vec();
    }
    out
}

/// Let `f` edit the data of every chunk in place; CRCs are recomputed afterwards.
pub fn foreach_png_chunk_mut(
    png: &mut [u8],
    mut f: impl FnMut(&[u8; 4], &mut [u8]),
) -> Result<(), anyhow::Error> {
    let spans: Vec<([u8; 4], usize, usize)> = png_chunks(png)
        .map(|chunk| (chunk.kind, chunk.start, chunk.end))
        .collect();
    for (kind, start, end) in spans {
        f(&kind, &mut png[start + 8..end - 4]);
        let crc = png_crc32(&png[start + 4..end - 4]); // CRC covers type + data
        png[end - 4..end].copy_from_slice(&crc.to_be_bytes());
    }
    Ok(())
}

pub fn png_crc32(data: &[u8]) -> u32 {
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

/// RIFF chunks of a WebP file as `(fourcc, payload)`.  Iteration stops at the
/// first chunk that does not fit in the file.
fn webp_chunks(webp: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let end = if is_webp(webp) {
        let riff_size = u32::from_le_bytes([webp[4], webp[5], webp[6], webp[7]]) as usize;
        8usize.saturating_add(riff_size).min(webp.len())
    } else {
        0
    };
    let mut pos = 12usize;
    std::iter::from_fn(move || {
        if pos + 8 > end {
            return None;
        }
        let fourcc = [webp[pos], webp[pos + 1], webp[pos + 2], webp[pos + 3]];
        let size = u32::from_le_bytes([webp[pos + 4], webp[pos + 5], webp[pos + 6], webp[pos + 7]])
            as usize;
        let payload = webp.get(pos + 8..(pos + 8).checked_add(size)?)?;
        pos += 8 + size + (size & 1); // chunks are padded to an even size
        Some((fourcc, payload))
    })
}

/// Rebuild a WebP file, optionally transforming each chunk.  Returning `None`
/// from `transform` drops the chunk.
pub fn rebuild_webp_chunks(
    webp: &[u8],
    transform: impl Fn(&[u8; 4], &[u8]) -> Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    if !is_webp(webp) {
        return None;
    }
    let mut out = Vec::with_capacity(webp.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&[0; 4]); // size, patched below
    out.extend_from_slice(b"WEBP");
    for (fourcc, payload) in webp_chunks(webp) {
        if let Some(new_payload) = transform(&fourcc, payload) {
            out.extend_from_slice(&fourcc);
            out.extend_from_slice(&(new_payload.len() as u32).to_le_bytes());
            out.extend_from_slice(&new_payload);
            if !new_payload.len().is_multiple_of(2) {
                out.push(0);
            }
        }
    }
    let riff_size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff_size.to_le_bytes());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Little-endian TIFF whose IFD0 holds one out-of-line UNDEFINED value of
    /// `blob_len` bytes (like a MakerNote).  Total length is `26 + blob_len`.
    fn tiff_with_blob(blob_len: usize) -> Vec<u8> {
        let mut t = b"II\x2A\x00\x08\x00\x00\x00".to_vec();
        t.extend_from_slice(&1u16.to_le_bytes());
        t.extend_from_slice(&0x927Cu16.to_le_bytes());
        t.extend_from_slice(&7u16.to_le_bytes());
        t.extend_from_slice(&(blob_len as u32).to_le_bytes());
        t.extend_from_slice(&26u32.to_le_bytes());
        t.extend_from_slice(&0u32.to_le_bytes());
        t.extend(std::iter::repeat_n(0xEEu8, blob_len));
        t
    }

    /// IFD0 -> ExifOffset -> Exif IFD holding a 100-byte value.  144 bytes long.
    fn tiff_with_exif_subifd() -> Vec<u8> {
        let mut t = b"II\x2A\x00\x08\x00\x00\x00".to_vec();
        t.extend_from_slice(&1u16.to_le_bytes());
        t.extend_from_slice(&[0x69, 0x87, 4, 0, 1, 0, 0, 0, 26, 0, 0, 0]);
        t.extend_from_slice(&[0; 4]);
        t.extend_from_slice(&1u16.to_le_bytes());
        t.extend_from_slice(&[0x7C, 0x92, 7, 0, 100, 0, 0, 0, 44, 0, 0, 0]);
        t.extend_from_slice(&[0; 4]);
        t.extend(std::iter::repeat_n(0xEEu8, 100));
        t
    }

    fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut c = (data.len() as u32).to_be_bytes().to_vec();
        c.extend_from_slice(kind);
        c.extend_from_slice(data);
        let crc = png_crc32(&c[4..]);
        c.extend_from_slice(&crc.to_be_bytes());
        c
    }

    fn webp_with(chunks: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut body = b"WEBP".to_vec();
        for (fourcc, payload) in chunks {
            body.extend_from_slice(fourcc);
            body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            body.extend_from_slice(payload);
            if payload.len() % 2 == 1 {
                body.push(0);
            }
        }
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend(body);
        out
    }

    #[test]
    fn tiff_extent_covers_out_of_line_values() {
        let t = tiff_with_blob(3000);
        assert_eq!(tiff_extent(&t), Some(26 + 3000));
        // Whatever follows the block in the surrounding file is not included.
        let mut padded = t.clone();
        padded.extend(std::iter::repeat_n(0x66u8, 64));
        assert_eq!(tiff_extent(&padded), Some(t.len()));
    }

    #[test]
    fn tiff_extent_follows_sub_ifds() {
        let t = tiff_with_exif_subifd();
        assert_eq!(t.len(), 144);
        assert_eq!(tiff_extent(&t), Some(144));
    }

    #[test]
    fn tiff_extent_survives_cycles_bad_pointers_and_non_tiff() {
        // IFD0 whose next-IFD pointer refers back to itself.
        let mut cyclic = b"II\x2A\x00\x08\x00\x00\x00".to_vec();
        cyclic.extend_from_slice(&0u16.to_le_bytes());
        cyclic.extend_from_slice(&8u32.to_le_bytes());
        assert_eq!(tiff_extent(&cyclic), Some(14));

        // A value that claims to live far beyond the end is clamped to the input.
        let mut t = tiff_with_blob(10);
        t[18..22].copy_from_slice(&0xFFFF_FF00u32.to_le_bytes());
        assert_eq!(tiff_extent(&t), Some(t.len()));

        assert_eq!(tiff_extent(b"not a tiff at all"), None);
        assert_eq!(tiff_extent(b"II\x2A\x00"), None);
    }

    #[test]
    fn read_short_tag_reads_both_byte_orders() {
        let le = [
            b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0,
            0,
        ];
        assert_eq!(read_short_tag_from_tiff(&le, 0x0112), Some(6));
        assert_eq!(read_short_tag_from_tiff(&le, 0x0110), None);
        let be = [
            b'M', b'M', 0, 42, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 8, 0, 0, 0, 0, 0,
            0,
        ];
        assert_eq!(read_short_tag_from_tiff(&be, 0x0112), Some(8));
        assert_eq!(read_short_tag_from_tiff(b"II", 0x0112), None);
    }

    #[test]
    fn jpeg_segments_handle_fill_bytes_standalone_markers_and_sos() {
        let jpeg = [
            0xFF, 0xD8, // SOI
            0xFF, 0xFF, 0xFF, 0xE0, 0x00, 0x04, b'J', b'F', // fill bytes, then APP0
            0xFF, 0x01, // TEM: standalone marker, no length
            0xFF, 0xE1, 0x00, 0x03, b'X', // APP1
            0xFF, 0xDA, 0x00, 0x02, // SOS: headers end here
            0xFF, 0xE1, 0x00, 0x03, b'Y', // must never be seen
        ];
        let segments: Vec<(u8, Vec<u8>)> = jpeg_segments(&jpeg)
            .map(|(marker, payload)| (marker, payload.to_vec()))
            .collect();
        assert_eq!(
            segments,
            vec![
                (0xE0, b"JF".to_vec()),
                (0x01, vec![]),
                (0xE1, b"X".to_vec())
            ]
        );
    }

    #[test]
    fn jpeg_truncated_or_corrupt_segments_do_not_panic() {
        // Segment length runs far past the end of the file.
        assert!(extract_exif_tiff(&[0xFF, 0xD8, 0xFF, 0xE1, 0xFF, 0xFF, b'E']).is_none());
        // Segment length below the minimum of 2.
        assert!(extract_exif_tiff(&[0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x01, 0, 0]).is_none());
        // Not a marker at all.
        assert!(extract_exif_tiff(&[0xFF, 0xD8, 0x12, 0x34, 0x56, 0x78]).is_none());
        assert!(extract_exif_tiff(&[0xFF, 0xD8]).is_none());
    }

    #[test]
    fn png_chunks_stop_at_a_truncated_chunk() {
        let mut png = PNG_SIG.to_vec();
        png.extend(png_chunk(b"tEXt", b"hi"));
        png.extend(png_chunk(b"IDAT", &[1, 2, 3, 4]));
        png.pop(); // chop the last byte of the final CRC

        let kinds: Vec<[u8; 4]> = png_chunks(&png).map(|c| c.kind).collect();
        assert_eq!(kinds, vec![*b"tEXt"]);
        // A damaged file is never rewritten.
        assert_eq!(rewrite_png_chunks(&png, |_, _| false), png);
    }

    #[test]
    fn png_rewrite_drops_unwanted_chunks_only() {
        let mut png = PNG_SIG.to_vec();
        png.extend(png_chunk(b"IHDR", &[0; 13]));
        png.extend(png_chunk(b"eXIf", b"II*\0"));
        png.extend(png_chunk(b"IEND", &[]));

        let out = rewrite_png_chunks(&png, |kind, _| kind != b"eXIf");
        let kinds: Vec<[u8; 4]> = png_chunks(&out).map(|c| c.kind).collect();
        assert_eq!(kinds, vec![*b"IHDR", *b"IEND"]);
    }

    #[test]
    fn png_foreach_mut_recomputes_crc() {
        let mut png = PNG_SIG.to_vec();
        png.extend(png_chunk(b"eXIf", b"abcd"));
        foreach_png_chunk_mut(&mut png, |kind, data| {
            if kind == b"eXIf" {
                data.fill(0);
            }
        })
        .unwrap();

        let chunk = png_chunks(&png).next().unwrap();
        assert_eq!(chunk.data, &[0u8; 4]);
        let stored = u32::from_be_bytes(png[chunk.end - 4..chunk.end].try_into().unwrap());
        assert_eq!(stored, png_crc32(&png[chunk.start + 4..chunk.end - 4]));
    }

    #[test]
    fn webp_chunks_respect_padding_and_rebuild_keeps_riff_size_correct() {
        let webp = webp_with(&[
            (*b"ABCD", b"xyz".to_vec()), // odd size => one pad byte
            (*b"EXIF", b"II*\0".to_vec()),
        ]);
        let found: Vec<([u8; 4], Vec<u8>)> = webp_chunks(&webp)
            .map(|(fourcc, payload)| (fourcc, payload.to_vec()))
            .collect();
        assert_eq!(
            found,
            vec![(*b"ABCD", b"xyz".to_vec()), (*b"EXIF", b"II*\0".to_vec())]
        );

        let rebuilt = rebuild_webp_chunks(&webp, |fourcc, payload| {
            (fourcc != b"ABCD").then(|| payload.to_vec())
        })
        .unwrap();
        assert!(is_webp(&rebuilt));
        let riff_size = u32::from_le_bytes(rebuilt[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff_size, rebuilt.len() - 8);
        let kinds: Vec<[u8; 4]> = webp_chunks(&rebuilt).map(|(f, _)| f).collect();
        assert_eq!(kinds, vec![*b"EXIF"]);
    }

    #[test]
    fn webp_chunk_larger_than_file_ends_iteration() {
        let mut webp = webp_with(&[(*b"EXIF", b"II*\0".to_vec())]);
        webp[16..20].copy_from_slice(&0x00FF_FFFFu32.to_le_bytes()); // absurd chunk size
        assert_eq!(webp_chunks(&webp).count(), 0);
        assert!(extract_exif_tiff(&webp).is_none());
    }
}
