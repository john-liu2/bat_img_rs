// HEIC‑specific EXIF handling (box parsing, extraction, replacement).
// Copyright © 2026 - Present, John Liu
//
// A HEIC file stores its EXIF one of two ways:
//   * inside an `Exif` box in `meta` (what simple writers and our test fixtures
//     produce), whose payload is exactly the TIFF block, or
//   * as an item in `mdat` (what real cameras and libheif produce), which has
//     no box around it and is found by scanning for the TIFF header.
// `locate_exif` hides that difference behind one question: "where are the bytes?"

use super::container::{EXIF_HEADER, is_tiff, tiff_extent};
use anyhow::{Result, anyhow, ensure};
use std::ops::Range;

/// Bytes always blanked after a TIFF header found outside an `Exif` box, even
/// when the header's own structure looks shorter.
const MIN_BLANK_WINDOW: usize = 1024;

// ---- public API ------------------------------------------------------------

/// Extract raw TIFF bytes from a HEIC file.  For an `Exif` box that is the
/// box payload; otherwise everything from the first TIFF header to the end of
/// the file (use `extract_heic_exif_tiff` for the exact block).
pub fn extract_heic_exif_raw(bytes: &[u8]) -> Option<Vec<u8>> {
    let (range, _) = locate_exif(bytes)?;
    Some(bytes[range].to_vec())
}

/// Like `extract_heic_exif_raw`, but the block is cut to the extent of the
/// TIFF structure itself, so it does not drag the image data along.
pub fn extract_heic_exif_tiff(bytes: &[u8]) -> Option<Vec<u8>> {
    let (range, exact) = locate_exif(bytes)?;
    let tiff = &bytes[range];
    let len = if exact {
        tiff.len()
    } else {
        tiff_extent(tiff)?
    };
    Some(tiff[..len].to_vec())
}

/// Overwrite the EXIF TIFF block with `new_tiff`, keeping the file length and
/// every box size unchanged.  When the block has a known size (an `Exif` box)
/// the unused tail is zeroed.  `new_tiff` must fit.
pub fn replace_heic_exif_payload(bytes: &[u8], new_tiff: &[u8]) -> Result<Vec<u8>> {
    let (range, exact) =
        locate_exif(bytes).ok_or_else(|| anyhow!("No Exif or TIFF header found"))?;
    ensure!(
        new_tiff.len() <= range.len(),
        "new EXIF block ({} bytes) does not fit in the {} bytes available",
        new_tiff.len(),
        range.len()
    );
    let mut out = bytes.to_vec();
    let end = range.start + new_tiff.len();
    out[range.start..end].copy_from_slice(new_tiff);
    if exact {
        out[end..range.end].fill(0);
    }
    Ok(out)
}

/// Return the TIFF payload from a libheif EXIF metadata block (handles `Exif\0\0` prefix).
pub fn tiff_from_heic_metadata(data: &[u8]) -> Option<&[u8]> {
    if data.starts_with(EXIF_HEADER) {
        return data.get(6..).filter(|tiff| is_tiff(tiff));
    }
    if is_tiff(data) {
        return Some(data);
    }
    let offset = u32::from_be_bytes(data.get(..4)?.try_into().ok()?) as usize;
    let tiff_start = 4usize.checked_add(offset)?;
    data.get(tiff_start..).filter(|tiff| is_tiff(tiff))
}

/// Complete HEIC metadata removal: blank every EXIF block and rename `Exif`
/// boxes / item types inside `meta` to `free`.  File length is unchanged.
pub fn strip_all_heic_metadata(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = bytes.to_vec();

    // Exif box: the payload is exactly the block.
    if let Some(payload) = exif_box_payload(&out) {
        out[payload].fill(0);
    }
    // Exif item in `mdat`: blank each remaining TIFF block over its full extent.
    while let Some(start) = find_tiff_header(&out) {
        let len = tiff_extent(&out[start..])
            .unwrap_or(0)
            .max(MIN_BLANK_WINDOW);
        let end = start.saturating_add(len).min(out.len());
        out[start..end].fill(0);
    }
    if let Some(meta) = meta_payload(&out) {
        for i in meta.start..meta.end.saturating_sub(3) {
            if &out[i..i + 4] == b"Exif" {
                out[i..i + 4].copy_from_slice(b"free");
            }
        }
    }
    Ok(out)
}

/// Robust scan for a TIFF header.  A header announced by the `Exif\0\0`
/// prefix wins over a bare `II*\0` / `MM\0*` that may occur by chance.
pub fn find_tiff_header(bytes: &[u8]) -> Option<usize> {
    let prefixed = bytes
        .windows(EXIF_HEADER.len() + 4)
        .position(|w| w.starts_with(EXIF_HEADER) && is_tiff(&w[EXIF_HEADER.len()..]))
        .map(|i| i + EXIF_HEADER.len());
    prefixed.or_else(|| bytes.windows(4).position(is_tiff))
}

// ---- locating the EXIF bytes -------------------------------------------------

/// Where the EXIF TIFF block lives, and whether its end is known exactly
/// (`true` for an `Exif` box; `false` when we only know where it starts and
/// the range therefore runs to the end of the file).
fn locate_exif(bytes: &[u8]) -> Option<(Range<usize>, bool)> {
    if let Some(payload) = exif_box_payload(bytes) {
        return Some((payload, true));
    }
    let start = find_tiff_header(bytes)?;
    Some((start..bytes.len(), false))
}

/// Payload (without the FullBox version/flags) of the `Exif` box inside `meta`.
fn exif_box_payload(bytes: &[u8]) -> Option<Range<usize>> {
    let meta = meta_payload(bytes)?;
    // `meta` is a FullBox: 4 bytes of version/flags precede its child boxes.
    let (_, start, end) = boxes(bytes, meta.start + 4, meta.end).find(|b| &b.0 == b"Exif")?;
    // `Exif` is a FullBox too.
    (start + 4 <= end).then(|| start + 4..end)
}

/// Payload range of the top-level `meta` box.
fn meta_payload(bytes: &[u8]) -> Option<Range<usize>> {
    boxes(bytes, 0, bytes.len())
        .find(|b| &b.0 == b"meta")
        .map(|(_, start, end)| start..end)
}

// ---- ISO-BMFF boxes -----------------------------------------------------------

/// A box as `(type, payload start, end)`, offsets into the file.
type BmffBox = ([u8; 4], usize, usize);

/// Read the box at `pos`, requiring it to lie entirely inside `..limit`
/// (`limit` must not exceed `bytes.len()`).  Handles the 64-bit "largesize"
/// form (size field == 1) and "extends to the end" (size field == 0).
fn read_box(bytes: &[u8], pos: usize, limit: usize) -> Option<BmffBox> {
    if pos.checked_add(8)? > limit {
        return None;
    }
    let head = bytes.get(pos..pos + 8)?;
    let size32 = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    let kind = [head[4], head[5], head[6], head[7]];
    let (header_len, size) = match size32 {
        0 => (8, limit - pos),
        1 => {
            let large = u64::from_be_bytes(bytes.get(pos + 8..pos + 16)?.try_into().ok()?);
            (16, usize::try_from(large).ok()?)
        }
        n => (8, n),
    };
    let end = pos.checked_add(size)?;
    (size >= header_len && end <= limit).then_some((kind, pos + header_len, end))
}

/// Sibling boxes in `start..limit`.  Stops at the first malformed box.
fn boxes(bytes: &[u8], start: usize, limit: usize) -> impl Iterator<Item = BmffBox> + '_ {
    let mut pos = start;
    std::iter::from_fn(move || {
        let b = read_box(bytes, pos, limit)?;
        pos = b.2;
        Some(b)
    })
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

    fn bmff_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut b = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(payload);
        b
    }

    /// ftyp + meta{ Exif box } + mdat, the layout the crate's fixtures use.
    fn heic_with_exif_box(tiff: &[u8]) -> Vec<u8> {
        let mut exif_payload = vec![0u8; 4]; // version/flags
        exif_payload.extend_from_slice(tiff);
        let mut meta_payload = vec![0u8; 4]; // version/flags
        meta_payload.extend(bmff_box(b"Exif", &exif_payload));
        let mut file = bmff_box(b"ftyp", b"heic\0\0\0\0mif1heic");
        file.extend(bmff_box(b"meta", &meta_payload));
        file.extend(bmff_box(b"mdat", &[0x55; 16]));
        file
    }

    /// ftyp + mdat holding `Exif\0\0` + `tiff` between two runs of filler, the
    /// way real cameras store it (no `Exif` box).  Returns (file, tiff offset).
    fn heic_with_exif_item(tiff: &[u8]) -> (Vec<u8>, usize) {
        let mut payload = vec![0x55u8; 64];
        payload.extend_from_slice(EXIF_HEADER);
        let tiff_offset_in_payload = payload.len();
        payload.extend_from_slice(tiff);
        payload.extend(std::iter::repeat_n(0x66u8, 64));
        let mut file = bmff_box(b"ftyp", b"heic\0\0\0\0mif1heic");
        let tiff_offset = file.len() + 8 + tiff_offset_in_payload;
        file.extend(bmff_box(b"mdat", &payload));
        (file, tiff_offset)
    }

    #[test]
    fn boxes_support_64bit_and_to_end_of_file_sizes() {
        let mut f = bmff_box(b"ftyp", b"heic");
        let mut big = 1u32.to_be_bytes().to_vec(); // "largesize" form
        big.extend_from_slice(b"free");
        big.extend_from_slice(&20u64.to_be_bytes());
        big.extend_from_slice(&[0; 4]);
        f.extend(big);
        let mut last = 0u32.to_be_bytes().to_vec(); // runs to the end of the file
        last.extend_from_slice(b"mdat");
        last.extend_from_slice(&[7; 5]);
        f.extend(last);

        let kinds: Vec<[u8; 4]> = boxes(&f, 0, f.len()).map(|b| b.0).collect();
        assert_eq!(kinds, vec![*b"ftyp", *b"free", *b"mdat"]);
        let (_, payload, end) = boxes(&f, 0, f.len()).last().unwrap();
        assert_eq!((payload, end), (f.len() - 5, f.len()));
    }

    #[test]
    fn boxes_reject_lying_or_tiny_sizes() {
        // Claims 256 bytes, has 8.
        assert_eq!(
            boxes(&[0, 0, 1, 0, b'm', b'e', b't', b'a'], 0, 8).count(),
            0
        );
        // Size smaller than its own header.
        assert_eq!(
            boxes(&[0, 0, 0, 4, b'm', b'e', b't', b'a'], 0, 8).count(),
            0
        );
        // A meta box too small to hold its version/flags never yields an Exif box.
        let meta_only = bmff_box(b"meta", &[]);
        assert_eq!(exif_box_payload(&meta_only), None);
    }

    #[test]
    fn exif_box_smaller_than_a_fullbox_header_does_not_underflow() {
        // Exif box with an 8-byte size (no room for version/flags).
        let mut meta_payload = vec![0u8; 4];
        meta_payload.extend(bmff_box(b"Exif", &[]));
        let file = bmff_box(b"meta", &meta_payload);
        assert_eq!(exif_box_payload(&file), None);
        assert!(extract_heic_exif_raw(&file).is_none());
    }

    #[test]
    fn find_tiff_header_prefers_the_exif_prefixed_block() {
        let mut data = vec![0u8; 16];
        data.extend_from_slice(b"II\x2A\x00\x01\x02\x03\x04"); // stray magic at 16
        assert_eq!(find_tiff_header(&data), Some(16));

        data.extend_from_slice(EXIF_HEADER);
        let real = data.len();
        data.extend_from_slice(b"MM\x00\x2A\x00\x00\x00\x08\x00\x00\x00\x00");
        assert_eq!(find_tiff_header(&data), Some(real));

        assert_eq!(find_tiff_header(b"nothing to see here"), None);
        assert_eq!(find_tiff_header(b"II"), None);
    }

    #[test]
    fn extract_covers_box_and_item_layouts() {
        let tiff = tiff_with_blob(200);

        let boxed = heic_with_exif_box(&tiff);
        assert_eq!(extract_heic_exif_raw(&boxed).unwrap(), tiff);
        assert_eq!(extract_heic_exif_tiff(&boxed).unwrap(), tiff);

        let (item, offset) = heic_with_exif_item(&tiff);
        // Raw: from the header to the end of the file, image data and all.
        assert_eq!(extract_heic_exif_raw(&item).unwrap(), item[offset..]);
        // Exact: just the TIFF block.
        assert_eq!(extract_heic_exif_tiff(&item).unwrap(), tiff);
    }

    #[test]
    fn replace_rejects_a_block_that_does_not_fit() {
        let tiff = tiff_with_blob(16);
        let heic = heic_with_exif_box(&tiff);
        let mut longer = tiff.clone();
        longer.push(0);
        assert!(replace_heic_exif_payload(&heic, &longer).is_err());
        assert!(replace_heic_exif_payload(b"no exif here", &tiff).is_err());
    }

    #[test]
    fn replace_shorter_block_zero_pads_inside_the_box_and_keeps_layout() {
        let tiff = tiff_with_blob(16);
        let heic = heic_with_exif_box(&tiff);
        let range = exif_box_payload(&heic).unwrap();
        let shorter = &tiff[..tiff.len() - 4];

        let out = replace_heic_exif_payload(&heic, shorter).unwrap();

        assert_eq!(out.len(), heic.len());
        assert_eq!(exif_box_payload(&out), Some(range.clone())); // box sizes untouched
        assert_eq!(&out[range.start..range.end - 4], shorter);
        assert_eq!(&out[range.end - 4..range.end], &[0u8; 4]);
        assert_eq!(&out[range.end..], &heic[range.end..]); // nothing outside the box moved
    }

    #[test]
    fn replace_in_item_layout_never_touches_bytes_after_the_new_block() {
        let tiff = tiff_with_blob(200);
        let (heic, offset) = heic_with_exif_item(&tiff);
        let shorter = &tiff[..10];

        let out = replace_heic_exif_payload(&heic, shorter).unwrap();

        assert_eq!(out.len(), heic.len());
        assert_eq!(&out[offset..offset + 10], shorter);
        assert_eq!(&out[offset + 10..], &heic[offset + 10..]); // no zero fill to EOF
    }

    #[test]
    fn strip_all_blanks_an_item_larger_than_the_minimum_window_and_nothing_more() {
        let tiff = tiff_with_blob(3000); // well past MIN_BLANK_WINDOW
        let (heic, offset) = heic_with_exif_item(&tiff);

        let out = strip_all_heic_metadata(&heic).unwrap();

        assert_eq!(out.len(), heic.len());
        assert!(!out.contains(&0xEE), "part of the EXIF block survived");
        assert!(out[offset..offset + tiff.len()].iter().all(|&b| b == 0));
        assert_eq!(&out[..offset], &heic[..offset]); // leading image bytes intact
        assert_eq!(&out[offset + tiff.len()..], &heic[offset + tiff.len()..]); // trailing too
        assert_eq!(find_tiff_header(&out), None);
    }

    #[test]
    fn strip_all_zeroes_the_exif_box_and_renames_it() {
        let tiff = tiff_with_blob(16);
        let heic = heic_with_exif_box(&tiff);

        let out = strip_all_heic_metadata(&heic).unwrap();

        assert_eq!(out.len(), heic.len());
        assert!(!out.windows(4).any(|w| w == b"Exif"));
        assert!(out.windows(4).any(|w| w == b"free"));
        assert!(out.windows(4).any(|w| w == b"meta"));
        assert_eq!(exif_box_payload(&out), None);
        assert_eq!(find_tiff_header(&out), None);
    }

    #[test]
    fn tiff_from_heic_metadata_handles_all_three_layouts() {
        let tiff = tiff_with_blob(4);
        let mut prefixed = EXIF_HEADER.to_vec();
        prefixed.extend_from_slice(&tiff);
        assert_eq!(tiff_from_heic_metadata(&prefixed), Some(tiff.as_slice()));
        assert_eq!(tiff_from_heic_metadata(&tiff), Some(tiff.as_slice()));

        let mut with_offset = 2u32.to_be_bytes().to_vec(); // skip two junk bytes
        with_offset.extend_from_slice(b"xx");
        with_offset.extend_from_slice(&tiff);
        assert_eq!(tiff_from_heic_metadata(&with_offset), Some(tiff.as_slice()));

        assert_eq!(tiff_from_heic_metadata(b"invalid"), None);
        assert_eq!(tiff_from_heic_metadata(&[]), None);
    }
}
