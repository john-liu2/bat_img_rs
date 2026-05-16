use anyhow::{Context, Result};
use image::{DynamicImage, GenericImage, GenericImageView, ImageBuffer, Rgba, RgbaImage};
use std::path::PathBuf;
use std::sync::Arc;

use crate::cli::Args;
use crate::error::BatImgError;
use crate::exif;
use crate::heic;
use crate::pipeline::Pipeline;

pub struct ProcessingContext {
    pub input_path: PathBuf,
    pub args: Arc<Args>,
    pub pipeline: Arc<Pipeline>,
}

impl ProcessingContext {
    pub fn process(&self) -> Result<PathBuf> {
        let p = &self.pipeline;
        let input = &self.input_path;

        // ── Determine output path ────────────────────────────────────────────
        let output_path = self.output_path()?;

        if p.dry_run {
            if !self.args.quiet {
                println!(
                    "  [dry-run] {} → {}",
                    input.display(),
                    output_path.display()
                );
            }
            return Ok(output_path);
        }

        if !p.overwrite && output_path.exists() {
            log::debug!("Skipping existing file: {}", output_path.display());
            return Ok(output_path);
        }

        // ── Read raw bytes (needed for EXIF before decode) ───────────────────
        // For HEIC we skip the raw-bytes path and let libheif handle everything.
        let is_heic = heic::is_heic(input);

        // ── Decode image + collect EXIF bytes ────────────────────────────────
        let (mut img, maybe_exif) = if is_heic {
            // libheif decodes and returns embedded EXIF in one call.
            heic::decode(input)
                .with_context(|| format!("Cannot decode HEIC: {}", input.display()))?
        } else {
            let raw_bytes = std::fs::read(input)
                .with_context(|| format!("Cannot read {}", input.display()))?;
            let img = image::load_from_memory(&raw_bytes)
                .with_context(|| format!("Cannot decode image: {}", input.display()))?;
            (img, Some(raw_bytes))
        };

        // ── Resolve the byte buffer used for EXIF operations ─────────────────
        // For HEIC, `maybe_exif` holds the bare EXIF block (no JPEG framing).
        // For other formats it holds the full raw file bytes.
        let raw_bytes_for_exif: Vec<u8> = maybe_exif.unwrap_or_default();

        // ── Handle EXIF: auto-orient, strip GPS / all ────────────────────────
        // For HEIC the pixel data already comes out of libheif correctly oriented
        // (libheif applies the transformation grid internally), so we only need
        // orientation for --auto-orient on non-HEIC files.
        let exif_orientation = if !is_heic && (p.auto_orient || p.strip_gps || p.strip_all) {
            exif::read_orientation(&raw_bytes_for_exif).unwrap_or(1)
        } else {
            1
        };

        // Strip metadata from non-HEIC files (HEIC pixels are already clean after
        // re-encoding to JPEG/PNG/WebP; no in-place EXIF rewrite is needed).
        let processed_bytes: Option<Vec<u8>> = if !is_heic {
            if p.strip_all {
                Some(exif::strip_all_metadata(&raw_bytes_for_exif)?)
            } else if p.strip_gps {
                Some(exif::strip_gps_metadata(&raw_bytes_for_exif)?)
            } else {
                None // use already-decoded img
            }
        } else {
            None
        };

        // Re-decode if we rewrote the bytes (non-HEIC strip path)
        if let Some(ref stripped) = processed_bytes {
            img = image::load_from_memory(stripped)
                .or_else(|_| image::load_from_memory(&raw_bytes_for_exif))
                .with_context(|| format!("Cannot decode stripped image: {}", input.display()))?;
        }

        // ── Auto-orient from EXIF ────────────────────────────────────────────
        if p.auto_orient {
            img = apply_orientation(img, exif_orientation);
        }

        // ── Grayscale ────────────────────────────────────────────────────────
        if p.grayscale {
            img = DynamicImage::ImageLuma8(img.to_luma8());
        }

        // ── Resize ───────────────────────────────────────────────────────────
        if let Some(ref spec) = p.resize {
            let (orig_w, orig_h) = img.dimensions();
            let (target_w, target_h) = resolve_dimensions(orig_w, orig_h, spec.width, spec.height);

            let skip = p.no_upscale && target_w > orig_w && target_h > orig_h;
            if !skip {
                img = img.resize(target_w, target_h, p.filter);
            }
        }

        // ── Brightness / Contrast ────────────────────────────────────────────
        if let Some(b) = p.brightness {
            img = img.brighten(b);
        }
        if let Some(c) = p.contrast {
            img = DynamicImage::ImageRgba8(
                image::imageops::contrast(&img.to_rgba8(), c)
            );
        }

        // ── Sharpen ──────────────────────────────────────────────────────────
        if p.sharpen {
            img = DynamicImage::ImageRgba8(
                image::imageops::unsharpen(&img.to_rgba8(), 1.0, 10)
            );
        }

        // ── Rotate ───────────────────────────────────────────────────────────
        if let Some(rot) = p.rotate {
            img = match rot {
                90 => img.rotate90(),
                180 => img.rotate180(),
                270 => img.rotate270(),
                _ => img,
            };
        }

        // ── Flip ─────────────────────────────────────────────────────────────
        if p.flip_h {
            img = img.fliph();
        }
        if p.flip_v {
            img = img.flipv();
        }

        // ── Border ───────────────────────────────────────────────────────────
        if let (Some(border_px), Some(color)) = (p.border_px, p.border_rgba) {
            img = add_border(img, border_px, color);
        }

        // ── Encode & save ────────────────────────────────────────────────────
        self.save_image(&img, &output_path)?;

        Ok(output_path)
    }

    fn output_path(&self) -> Result<PathBuf> {
        let p = &self.pipeline;
        let stem = self
            .input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("image");

        let ext = if let Some(fmt) = p.output_format {
            fmt.extension().to_string()
        } else {
            let src_ext = self
                .input_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("jpg")
                .to_lowercase();

            // HEIC/HEIF cannot be re-encoded by this tool — default output to JPEG.
            // The user can override with -f / --format.
            match src_ext.as_str() {
                "heic" | "heif" => "jpg".to_string(),
                other => other.to_string(),
            }
        };

        let filename = format!("{}{}{}.{}", p.prefix, stem, p.suffix, ext);
        Ok(p.output_dir.join(filename))
    }

    fn save_image(&self, img: &DynamicImage, path: &PathBuf) -> Result<()> {
        let p = &self.pipeline;
        let quality = p.quality;

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("jpg")
            .to_lowercase();

        match ext.as_str() {
            "jpg" | "jpeg" => {
                let mut out = std::fs::File::create(path)
                    .with_context(|| format!("Cannot create {}", path.display()))?;
                let mut encoder =
                    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
                encoder
                    .encode_image(img)
                    .with_context(|| format!("JPEG encode failed for {}", path.display()))?;
            }
            "webp" => {
                // image crate WebP encoding (lossless for PNG source, lossy for JPEG)
                img.save(path)
                    .with_context(|| format!("WebP save failed for {}", path.display()))?;
            }
            _ => {
                img.save(path)
                    .with_context(|| format!("Save failed for {}", path.display()))?;
            }
        }

        Ok(())
    }
}

// ── Helpers ────────────────────────────────────────────

/// Resolve target dimensions; 0 means "auto from aspect ratio".
fn resolve_dimensions(orig_w: u32, orig_h: u32, target_w: u32, target_h: u32) -> (u32, u32) {
    match (target_w, target_h) {
        (0, 0) => (orig_w, orig_h),
        (w, 0) => {
            let ratio = w as f64 / orig_w as f64;
            (w, (orig_h as f64 * ratio).round() as u32)
        }
        (0, h) => {
            let ratio = h as f64 / orig_h as f64;
            ((orig_w as f64 * ratio).round() as u32, h)
        }
        (w, h) => (w, h),
    }
}

/// Add a solid-color border around an image.
fn add_border(img: DynamicImage, px: u32, color: Rgba<u8>) -> DynamicImage {
    let (w, h) = img.dimensions();
    let new_w = w + px * 2;
    let new_h = h + px * 2;
    let mut canvas: RgbaImage = ImageBuffer::from_pixel(new_w, new_h, color);
    image::imageops::overlay(&mut canvas, &img.to_rgba8(), px as i64, px as i64);
    DynamicImage::ImageRgba8(canvas)
}

/// Apply EXIF orientation to a decoded image.
fn apply_orientation(img: DynamicImage, orientation: u32) -> DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}
