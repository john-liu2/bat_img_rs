//! HEIC / HEIF decoding via `libheif-rs` (wraps the native `libheif` C library).
//!
//! ## System requirements (macOS)
//!
//!   brew install libheif
//!
//! `libheif` brings in `libde265` (H.265 decoder) and optionally `libaom`
//! (AV1/AVIF) as transitive dependencies.
//!
//! ## What this module does
//!
//! 1. Detects whether an image path is HEIC/HEIF by extension.
//! 2. Decodes the primary image item to an interleaved RGB or RGBA byte buffer
//!    using `libheif-rs`.
//! 3. Wraps the raw buffer into an `image::DynamicImage` so the rest of the
//!    pipeline can treat it identically to any other format.
//! 4. Extracts the raw EXIF block (if present) so the caller can pass it to
//!    the existing `exif::*` helpers for GPS-stripping / orientation.

use anyhow::{bail, Context, Result};
use image::{DynamicImage, RgbImage, RgbaImage};
use libheif_rs::{
    Channel, ColorSpace, HeifContext, LibHeif, RgbChroma,
};
use std::path::Path;

/// Returns `true` when the file extension indicates HEIC or HEIF.
pub fn is_heic(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("heic") | Some("heif")
    )
}

/// Decode a HEIC/HEIF file into a [`DynamicImage`].
///
/// Also returns the raw EXIF bytes (if the file embeds them) so the caller
/// can forward them to [`crate::exif::read_orientation`] etc.
pub fn decode(path: &Path) -> Result<(DynamicImage, Option<Vec<u8>>)> {
    let lib = LibHeif::new();

    // ── Load context from file ───────────────────────────────────────────────
    let ctx = HeifContext::read_from_file(
        path.to_str()
            .context("HEIC path is not valid UTF-8")?,
    )
    .with_context(|| format!("libheif: cannot open {}", path.display()))?;

    // ── Primary image item ───────────────────────────────────────────────────
    let handle = ctx
        .primary_image_handle()
        .context("libheif: no primary image in HEIC file")?;

    // ── Extract EXIF metadata (best-effort) ──────────────────────────────────
    // metadata_block_ids(item_ids: &mut [ItemId], type_filter) -> usize
    // We pre-allocate one slot; the return value is how many were written.
    let exif_bytes: Option<Vec<u8>> = {
        let mut id_buf: [u32; 1] = [0];
        let count = handle.metadata_block_ids(&mut id_buf, b"Exif");
        if count == 0 {
            None
        } else {
            handle
                .metadata(id_buf[0])
                .ok()
                .map(|raw| {
                    // libheif prefixes EXIF blocks with a 4-byte offset box;
                    // skip it to get a bare TIFF/EXIF block.
                    if raw.len() > 4 {
                        raw[4..].to_vec()
                    } else {
                        raw
                    }
                })
        }
    };

    // ── Decide colour space: RGBA if image has alpha, RGB otherwise ──────────
    let has_alpha = handle.has_alpha_channel();

    let img: DynamicImage = if has_alpha {
        let decoded = lib
            .decode(&handle, ColorSpace::Rgb(RgbChroma::Rgba), None)
            .context("libheif: RGBA decode failed")?;

        let plane = decoded
            .planes()
            .interleaved
            .context("libheif: no interleaved RGBA plane")?;

        let width = plane.width;
        let height = plane.height;
        let stride = plane.stride; // bytes per row (may include padding)

        // Copy row-by-row to strip any stride padding
        let mut pixels: Vec<u8> = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height as usize {
            let start = row * stride;
            let end = start + width as usize * 4;
            pixels.extend_from_slice(&plane.data[start..end]);
        }

        let buf = RgbaImage::from_raw(width, height, pixels)
            .context("libheif: cannot build RgbaImage from decoded data")?;
        DynamicImage::ImageRgba8(buf)
    } else {
        let decoded = lib
            .decode(&handle, ColorSpace::Rgb(RgbChroma::Rgb), None)
            .context("libheif: RGB decode failed")?;

        let plane = decoded
            .planes()
            .interleaved
            .context("libheif: no interleaved RGB plane")?;

        let width = plane.width;
        let height = plane.height;
        let stride = plane.stride;

        let mut pixels: Vec<u8> = Vec::with_capacity((width * height * 3) as usize);
        for row in 0..height as usize {
            let start = row * stride;
            let end = start + width as usize * 3;
            pixels.extend_from_slice(&plane.data[start..end]);
        }

        let buf = RgbImage::from_raw(width, height, pixels)
            .context("libheif: cannot build RgbImage from decoded data")?;
        DynamicImage::ImageRgb8(buf)
    };

    Ok((img, exif_bytes))
}
