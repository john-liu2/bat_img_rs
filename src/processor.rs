use crate::exif;
use crate::heic;
use crate::pipeline::Pipeline;
use anyhow::{Context, Result};
use image::{DynamicImage, GenericImageView, ImageBuffer, Rgba, RgbaImage};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

pub struct ProcessingContext {
    pub input_path: PathBuf,
    pub pipeline: Arc<Pipeline>,
}

impl ProcessingContext {
    /// Fast path: operates directly on raw file bytes without decoding/re-encoding pixels.
    fn process_metadata_fast_path(&self, raw_bytes: Vec<u8>) -> Result<std::path::PathBuf> {
        let target_path = self.output_path()?;
        if self.pipeline.dry_run {
            return Ok(target_path);
        }

        let cleaned_bytes = if self.pipeline.strip_all {
            exif::strip_all_metadata(&raw_bytes)?
        } else if self.pipeline.strip_gps {
            exif::strip_gps_metadata(&raw_bytes)?
        } else {
            raw_bytes
        };

        let out_dir = target_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        let mut temp_file = tempfile::NamedTempFile::new_in(out_dir)?;
        std::io::Write::write_all(&mut temp_file, &cleaned_bytes)?;
        temp_file
            .persist(&target_path)
            .with_context(|| format!("Failed to write output file: {}", target_path.display()))?;

        Ok(target_path)
    }

    pub fn process(&self) -> Result<PathBuf> {
        let p = &self.pipeline;
        // Detect if this job is strictly a metadata stripping operation
        let is_metadata_only = (p.strip_all || p.strip_gps)
            && p.resize.is_none()
            && p.rotate.is_none()
            && !p.flip_h
            && !p.flip_v
            && p.border_px.is_none()
            && p.brightness.is_none()
            && p.contrast.is_none()
            && !p.sharpen
            && !p.grayscale
            && p.output_format.is_none();

        if is_metadata_only {
            let raw_bytes = fs::read(&self.input_path).with_context(|| {
                format!("Failed to read input file: {}", self.input_path.display())
            })?;

            // Fast path for unoriented photos; fall back to full decoding if orientation needs pixel rotation
            let has_orientation = exif::read_orientation(&raw_bytes).is_some_and(|o| o != 1);
            if !has_orientation {
                return self.process_metadata_fast_path(raw_bytes);
            }
        }

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
            let raw_bytes =
                std::fs::read(input).with_context(|| format!("Cannot read {}", input.display()))?;
            let img = image::load_from_memory(&raw_bytes)
                .with_context(|| format!("Cannot decode image: {}", input.display()))?;
            (img, Some(raw_bytes))
        };

        // ── Resolve the byte buffer used for EXIF operations ─────────────────
        // For HEIC, `maybe_exif` holds the bare EXIF block (no JPEG framing).
        // For other formats it holds the full raw file bytes.
        let raw_bytes_for_exif: Vec<u8> = maybe_exif.unwrap_or_default();

        // For HEIC the pixel data already comes out of libheif correctly oriented
        // (libheif applies the transformation grid internally), so we only need
        // orientation for non-HEIC files.
        let exif_orientation = if !is_heic && (p.strip_gps || p.strip_all) {
            exif::read_orientation(&raw_bytes_for_exif).unwrap_or(1)
        } else {
            1
        };

        let processed_exif: Option<Vec<u8>> = if p.strip_all {
            None
        } else if p.strip_gps {
            Some(exif::strip_gps_metadata(&raw_bytes_for_exif)?)
        } else if raw_bytes_for_exif.is_empty() {
            None
        } else {
            Some(raw_bytes_for_exif.clone())
        };

        // ── Auto-orient from EXIF ────────────────────────────────────────────
        // Re-encoding JPEGs drops EXIF, so bake orientation into pixels whenever
        // we strip metadata.
        if p.strip_all || p.strip_gps {
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
            img = DynamicImage::ImageRgba8(image::imageops::contrast(&img.to_rgba8(), c));
        }

        // ── Sharpen ──────────────────────────────────────────────────────────
        if p.sharpen {
            img = DynamicImage::ImageRgba8(image::imageops::unsharpen(&img.to_rgba8(), 1.0, 10));
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
        self.save_image(
            &img,
            &output_path,
            heic_meta.as_ref(),
            processed_exif.as_deref(),
        )?;

        // Re-encoding for non-HEIC
        if !is_heic && let Some(ref exif_bytes) = processed_exif {
            exif::write_exif_file(&output_path, exif_bytes)?;
        }
        Ok(output_path)
    }

    fn output_path(&self) -> Result<PathBuf> {
        let p = &self.pipeline;

        // ── In-place mode: output = input path (same file, same format) ──────
        if p.in_place {
            // Disallow in-place when --format changes the extension, since that
            // would silently rename the file.  Require --output in that case.
            if let Some(fmt) = p.output_format {
                let src_ext = self
                    .input_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                let dst_ext = fmt.extension();
                // jpeg/jpg, png/png, tiff/tif, webp/webp etc. to allow in-place
                let same = src_ext == dst_ext
                    || (src_ext == "jpg" && dst_ext == "jpeg")
                    || (src_ext == "jpeg" && dst_ext == "jpg")
                    || (src_ext == "png" && dst_ext == "png")
                    || (src_ext == "tif" && dst_ext == "tiff")
                    || (src_ext == "tiff" && dst_ext == "tif")
                    || (src_ext == "webp" && dst_ext == "webp");
                if !same {
                    anyhow::bail!(
                        "In-place mode cannot change format from .{} to .{}. \
                         Please specify --output <DIR>.",
                        src_ext,
                        dst_ext
                    );
                }
            }
            return Ok(self.input_path.clone());
        }

        // ── Normal mode: write into output_dir ───────────────────────────────
        let out_dir = p
            .output_dir
            .as_ref()
            .expect("output_dir set when not in_place");
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
        exif: Option<&[u8]>,
    ) -> Result<()> {
        let p = &self.pipeline;
        let quality_or_default = p.quality.unwrap_or(90);

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("jpg")
            .to_lowercase();

        // When writing in-place, encode to a sibling temp file first,
        // then atomically rename over the original.  This guarantees the
        // original is never left in a half-written state if encoding fails.
        let (write_path, is_temp) = if p.in_place {
            let tmp = path.with_file_name(format!(
                "{}.{}.bat_img_tmp",
                path.file_stem().unwrap().to_string_lossy(),
                ext
            ));
            (tmp, true)
        } else {
            (path.clone(), false)
        };

        let encode_result =
            self.encode_to(&write_path, img, &ext, quality_or_default, heic_meta, exif);

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
        exif: Option<&[u8]>,
    ) -> Result<()> {
        let p = &self.pipeline;

        match ext {
            "heic" | "heif" => {
                use libheif_rs::CompressionFormat;
                let compression = heic_meta
                    .map(|m| m.compression)
                    .unwrap_or(CompressionFormat::Hevc);
                heic::encode(img, path, compression, p.quality, exif)
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
                img.save_with_format(path, image::ImageFormat::WebP)
                    .with_context(|| format!("WebP save failed for {}", path.display()))?;
            }
            "png" => {
                img.save_with_format(path, image::ImageFormat::Png)
                    .with_context(|| format!("PNG save failed for {}", path.display()))?;
            }
            "tif" | "tiff" => {
                img.save_with_format(path, image::ImageFormat::Tiff)
                    .with_context(|| format!("TIFF save failed for {}", path.display()))?;
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
