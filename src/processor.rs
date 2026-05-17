use anyhow::{Context, Result};
use image::{DynamicImage, GenericImageView, ImageBuffer, Rgba, RgbaImage};
use std::path::PathBuf;
use std::sync::Arc;

// use crate::error::BatImgError;
use crate::exif;
use crate::heic;
use crate::pipeline::Pipeline;

pub struct ProcessingContext {
    pub input_path: PathBuf,
    pub pipeline: Arc<Pipeline>,
}

impl ProcessingContext {
    pub fn process(&self) -> Result<PathBuf> {
        let p = &self.pipeline;
        let input = &self.input_path;

        // ── Determine output path ────────────────────────────────────────────
        let output_path = self.output_path()?;

        if p.dry_run {
            return Ok(output_path);
        }

        if !p.in_place && !p.overwrite && output_path.exists() {
            log::debug!("Skipping existing file: {}", output_path.display());
            return Ok(output_path);
        }

        // ── Read raw bytes (needed for EXIF before decode) ───────────────────
        // For HEIC we skip the raw-bytes path and let libheif handle everything.
        let is_heic = heic::is_heic(input);

        // ── Decode image + collect EXIF bytes + HEIC encoding metadata ────────
        let heic_meta;
        let (mut img, maybe_exif) = if is_heic {
            let (img, exif, meta) = heic::decode(input)
                .with_context(|| format!("Cannot decode HEIC: {}", input.display()))?;
            heic_meta = Some(meta);
            (img, exif)
        } else {
            heic_meta = None;
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
                // Both dimensions explicit → exact resize (may change aspect ratio).
                // One dimension was 0 → aspect-ratio-preserving resize.
                img = if spec.width != 0 && spec.height != 0 {
                    img.resize_exact(target_w, target_h, p.filter)
                } else {
                    img.resize(target_w, target_h, p.filter)
                };
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
        self.save_image(&img, &output_path, heic_meta.as_ref())?;

        Ok(output_path)
    }

    fn output_path(&self) -> Result<PathBuf> {
        let p = &self.pipeline;

        // ── In-place mode: output = input path (same file, same format) ──────
        if p.in_place {
            // Disallow in-place when --format changes the extension, since that
            // would silently rename the file.  Require --output in that case.
            if let Some(fmt) = p.output_format {
                let src_ext = self.input_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                let dst_ext = fmt.extension();
                // normalise jpeg/jpg
                let same = src_ext == dst_ext
                    || (src_ext == "jpg" && dst_ext == "jpeg")
                    || (src_ext == "jpeg" && dst_ext == "jpg");
                if !same {
                    anyhow::bail!(
                        "In-place mode cannot change format from .{} to .{}. \
                         Please specify --output <DIR>.",
                        src_ext, dst_ext
                    );
                }
            }
            return Ok(self.input_path.clone());
        }

        // ── Normal mode: write into output_dir ───────────────────────────────
        let out_dir = p.output_dir.as_ref().expect("output_dir set when not in_place");
        let stem = self
            .input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("image");

        let ext = if let Some(fmt) = p.output_format {
            fmt.extension().to_string()
        } else {
            self.input_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("jpg")
                .to_lowercase()
        };

        let filename = format!("{}{}{}.{}", p.prefix, stem, p.suffix, ext);
        Ok(out_dir.join(filename))
    }

    fn save_image(
        &self,
        img: &DynamicImage,
        path: &PathBuf,
        heic_meta: Option<&heic::HeicMeta>,
    ) -> Result<()> {
        let p = &self.pipeline;
        let quality_or_default = p.quality.unwrap_or(90);

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("jpg")
            .to_lowercase();

        // ── When writing in-place, encode to a sibling temp file first,
        //    then atomically rename over the original.  This guarantees the
        //    original is never left in a half-written state if encoding fails.
        let (write_path, is_temp) = if p.in_place {
            let tmp = path.with_extension(format!("{}.bat_img_tmp", ext));
            (tmp, true)
        } else {
            (path.clone(), false)
        };

        let encode_result = self.encode_to(&write_path, img, &ext, quality_or_default, heic_meta);

        if let Err(e) = encode_result {
            // Clean up the temp file if encoding failed
            if is_temp {
                let _ = std::fs::remove_file(&write_path);
            }
            return Err(e);
        }

        // Atomic rename: temp → original
        if is_temp {
            std::fs::rename(&write_path, path).with_context(|| {
                format!(
                    "Failed to rename temp file {} → {}",
                    write_path.display(),
                    path.display()
                )
            })?;
        }

        Ok(())
    }

    fn encode_to(
        &self,
        path: &PathBuf,
        img: &DynamicImage,
        ext: &str,
        quality: u8,
        heic_meta: Option<&heic::HeicMeta>,
    ) -> Result<()> {
        let p = &self.pipeline;

        match ext {
            "heic" | "heif" => {
                use libheif_rs::CompressionFormat;
                let compression = heic_meta
                    .map(|m| m.compression)
                    .unwrap_or(CompressionFormat::Hevc);
                heic::encode(img, path, compression, p.quality)
                    .with_context(|| format!("HEIC encode failed for {}", path.display()))?;
            }
            "jpg" | "jpeg" => {
                let rgb = img.to_rgb8();
                let mut out = std::fs::File::create(path)
                    .with_context(|| format!("Cannot create {}", path.display()))?;
                let mut encoder =
                    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
                encoder
                    .encode(
                        rgb.as_raw(),
                        rgb.width(),
                        rgb.height(),
                        image::ExtendedColorType::Rgb8,
                    )
                    .with_context(|| format!("JPEG encode failed for {}", path.display()))?;
            }
            "webp" => {
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

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Resolve target dimensions; 0 means "auto from aspect ratio".
pub fn resolve_dimensions(orig_w: u32, orig_h: u32, target_w: u32, target_h: u32) -> (u32, u32) {
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
