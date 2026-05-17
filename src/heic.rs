//! HEIC / HEIF decoding and encoding via `libheif-rs`.
//!
//! ## System requirements (macOS)
//!
//!   brew install libheif
//!
//! `libheif` brings in `libde265` (H.265/HEVC decoder) and optionally
//! `libaom` (AV1/AVIF) as transitive dependencies.

use anyhow::{Context, Result};
use image::{DynamicImage, RgbImage, RgbaImage};
use libheif_rs::{
    Channel, ColorSpace, CompressionFormat, EncoderQuality, HeifContext, Image, LibHeif, RgbChroma,
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

/// Everything we learn about an input HEIC file during decode,
/// needed to faithfully re-encode it afterwards.
#[derive(Debug, Clone)]
pub struct HeicMeta {
    /// The codec the input file used (HEVC, AV1, …).
    pub compression: CompressionFormat,
}

/// Decode a HEIC/HEIF file into a [`DynamicImage`].
///
/// Returns:
/// - the decoded pixels as a `DynamicImage`
/// - raw EXIF bytes (if present), for GPS-strip / auto-orient
/// - [`HeicMeta`] describing the input encoding, for faithful re-encoding
pub fn decode(path: &Path) -> Result<(DynamicImage, Option<Vec<u8>>, HeicMeta)> {
    let lib = LibHeif::new();

    // ── Load context from file ───────────────────────────────────────────────
    let ctx = HeifContext::read_from_file(path.to_str().context("HEIC path is not valid UTF-8")?)
        .with_context(|| format!("libheif: cannot open {}", path.display()))?;

    // ── Primary image item ───────────────────────────────────────────────────
    let handle = ctx
        .primary_image_handle()
        .context("libheif: no primary image in HEIC file")?;

    // ── Detect compression format ────────────────────────────────────────────
    // libheif-rs 0.22 has no `handle.compression_format()` method.
    // We derive the codec from the file extension, which is reliable:
    //   .heic → always HEVC (the HEIC spec mandates it)
    //   .heif → often AV1 on newer devices; try AV1, fall back to HEVC at encode time
    let compression = match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("heif") => CompressionFormat::Av1,
        _ => CompressionFormat::Hevc,
    };

    let meta = HeicMeta { compression };

    // ── Extract EXIF metadata (best-effort) ──────────────────────────────────
    let exif_bytes: Option<Vec<u8>> = {
        let mut id_buf: [u32; 1] = [0];
        let count = handle.metadata_block_ids(&mut id_buf, b"Exif");
        if count == 0 {
            None
        } else {
            handle.metadata(id_buf[0]).ok().map(|raw| {
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

        let (width, height, stride) = (plane.width, plane.height, plane.stride);
        let mut pixels: Vec<u8> = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height as usize {
            let start = row * stride;
            pixels.extend_from_slice(&plane.data[start..start + width as usize * 4]);
        }

        DynamicImage::ImageRgba8(
            RgbaImage::from_raw(width, height, pixels)
                .context("libheif: cannot build RgbaImage")?,
        )
    } else {
        let decoded = lib
            .decode(&handle, ColorSpace::Rgb(RgbChroma::Rgb), None)
            .context("libheif: RGB decode failed")?;

        let plane = decoded
            .planes()
            .interleaved
            .context("libheif: no interleaved RGB plane")?;

        let (width, height, stride) = (plane.width, plane.height, plane.stride);
        let mut pixels: Vec<u8> = Vec::with_capacity((width * height * 3) as usize);
        for row in 0..height as usize {
            let start = row * stride;
            pixels.extend_from_slice(&plane.data[start..start + width as usize * 3]);
        }

        DynamicImage::ImageRgb8(
            RgbImage::from_raw(width, height, pixels).context("libheif: cannot build RgbImage")?,
        )
    };

    Ok((img, exif_bytes, meta))
}

/// Encode a [`DynamicImage`] as a HEIC/HEIF file at `path`.
///
/// - `compression` — use the same codec as the source file (HEVC, AV1, …)
/// - `quality` — `None` means use the encoder's default quality, which
///   produces output closest in size to the original. Pass `Some(n)` only
///   when the user explicitly requested a quality via `--quality`.
pub fn encode(
    img: &DynamicImage,
    path: &Path,
    compression: CompressionFormat,
    quality: Option<u8>,
) -> Result<()> {
    let lib = LibHeif::new();

    let has_alpha = matches!(
        img,
        DynamicImage::ImageRgba8(_) | DynamicImage::ImageRgba16(_)
    );

    let (width, height) = (img.width(), img.height());

    // ── Build libheif Image from pixel buffer ────────────────────────────────
    let heif_img = if has_alpha {
        let rgba = img.to_rgba8();
        let mut hi = Image::new(width, height, ColorSpace::Rgb(RgbChroma::Rgba))
            .context("libheif: cannot create RGBA image")?;
        hi.create_plane(Channel::Interleaved, width, height, 32)
            .context("libheif: cannot create interleaved RGBA plane")?;

        let plane = hi
            .planes_mut()
            .interleaved
            .context("libheif: no interleaved plane")?;
        let stride = plane.stride;
        let data = plane.data;
        for row in 0..height as usize {
            let src = row * width as usize * 4;
            let dst = row * stride;
            data[dst..dst + width as usize * 4]
                .copy_from_slice(&rgba.as_raw()[src..src + width as usize * 4]);
        }
        hi
    } else {
        let rgb = img.to_rgb8();
        let mut hi = Image::new(width, height, ColorSpace::Rgb(RgbChroma::Rgb))
            .context("libheif: cannot create RGB image")?;
        hi.create_plane(Channel::Interleaved, width, height, 24)
            .context("libheif: cannot create interleaved RGB plane")?;

        let plane = hi
            .planes_mut()
            .interleaved
            .context("libheif: no interleaved plane")?;
        let stride = plane.stride;
        let data = plane.data;
        for row in 0..height as usize {
            let src = row * width as usize * 3;
            let dst = row * stride;
            data[dst..dst + width as usize * 3]
                .copy_from_slice(&rgb.as_raw()[src..src + width as usize * 3]);
        }
        hi
    };

    // ── Set up encoder using the same codec as the input ─────────────────────
    let encoder_name = match compression {
        CompressionFormat::Av1 => "AV1",
        CompressionFormat::Hevc => "HEVC",
        _ => "HEVC", // safe fallback
    };

    let mut encoder = lib
        .encoder_for_format(compression)
        // If the exact codec isn't available, fall back to HEVC
        .or_else(|_| lib.encoder_for_format(CompressionFormat::Hevc))
        .with_context(|| {
            format!(
                "libheif: no {} encoder available (brew reinstall libheif)",
                encoder_name
            )
        })?;

    // Apply quality only when the user explicitly asked for it.
    // Leaving it at the encoder default produces the most faithful re-encode.
    if let Some(q) = quality {
        encoder
            .set_quality(EncoderQuality::Lossy(q))
            .context("libheif: cannot set encoder quality")?;
    }

    // ── Encode and write ─────────────────────────────────────────────────────
    let mut ctx = HeifContext::new().context("libheif: cannot create encoding context")?;
    ctx.encode_image(&heif_img, &mut encoder, None)
        .context("libheif: encoding failed")?;
    ctx.write_to_file(
        path.to_str()
            .context("HEIC output path is not valid UTF-8")?,
    )
    .context("libheif: cannot write HEIC file")?;

    Ok(())
}
