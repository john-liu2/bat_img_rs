use anyhow::{Context, Result};
use glob::glob;
use std::path::PathBuf;
use walkdir::WalkDir;

use crate::cli::{Args, OutputFormat};
use crate::error::BatImgError;

/// Supported image extensions
const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "tiff", "tif", "bmp", "gif",
    "heic", "heif",
];

fn is_image(path: &PathBuf) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Collect all input image paths from the provided patterns/paths/directories.
pub fn collect_input_files(args: &Args) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = Vec::new();

    for input in &args.input {
        let path = PathBuf::from(input);

        if path.is_dir() {
            // Directory: walk (optionally recursive)
            let walker = if args.recursive {
                WalkDir::new(&path)
            } else {
                WalkDir::new(&path).max_depth(1)
            };

            for entry in walker.into_iter().filter_map(|e| e.ok()) {
                let p = entry.into_path();
                if p.is_file() && is_image(&p) {
                    files.push(p);
                }
            }
        } else if path.is_file() {
            if is_image(&path) {
                files.push(path);
            }
        } else {
            // Glob pattern
            let matches: Vec<PathBuf> = glob(input)
                .with_context(|| format!("Invalid glob pattern: {}", input))?
                .filter_map(|r| r.ok())
                .filter(|p| p.is_file() && is_image(p))
                .collect();

            if matches.is_empty() {
                eprintln!("Warning: no files matched pattern '{}'", input);
            }
            files.extend(matches);
        }
    }

    // Deduplicate while preserving order
    files.sort();
    files.dedup();

    Ok(files)
}

// ── Pipeline step definitions ────────────────────────

#[derive(Debug, Clone)]
pub struct ResizeSpec {
    pub width: u32,
    pub height: u32,
}

/// The parsed, validated pipeline — built once, shared across all threads.
#[derive(Debug, Clone)]
pub struct Pipeline {
    pub strip_gps: bool,
    pub strip_all: bool,
    pub auto_orient: bool,

    pub resize: Option<ResizeSpec>,
    pub no_upscale: bool,
    pub filter: image::imageops::FilterType,

    pub rotate: Option<u32>,
    pub flip_h: bool,
    pub flip_v: bool,

    pub border_px: Option<u32>,
    pub border_rgba: Option<image::Rgba<u8>>,

    pub brightness: Option<i32>,
    pub contrast: Option<f32>,
    pub sharpen: bool,
    pub grayscale: bool,

    pub output_format: Option<OutputFormat>,
    pub quality: u8,
    pub output_dir: std::path::PathBuf,
    pub prefix: String,
    pub suffix: String,
    pub overwrite: bool,
    pub dry_run: bool,
}

/// Parse and validate the CLI args into a reusable Pipeline.
pub fn build_pipeline(args: &Args) -> Result<Pipeline> {
    // ── Resize ───────────────────────────────────────
    let resize = if let Some(spec) = &args.resize {
        let parts: Vec<&str> = spec.splitn(2, 'x').collect();
        if parts.len() != 2 {
            return Err(BatImgError::InvalidResize(spec.clone()).into());
        }
        let w: u32 = parts[0]
            .parse()
            .map_err(|_| BatImgError::InvalidResize(spec.clone()))?;
        let h: u32 = parts[1]
            .parse()
            .map_err(|_| BatImgError::InvalidResize(spec.clone()))?;
        if w == 0 && h == 0 {
            return Err(BatImgError::InvalidResize(spec.clone()).into());
        }
        Some(ResizeSpec { width: w, height: h })
    } else {
        None
    };

    // ── Rotation ─────────────────────────────────────
    if let Some(rot) = args.rotate {
        if ![90, 180, 270].contains(&rot) {
            return Err(BatImgError::InvalidRotation(rot).into());
        }
    }

    // ── Border color ─────────────────────────────────
    let border_rgba = if args.border.is_some() {
        Some(parse_color(&args.border_color)?)
    } else {
        None
    };

    // ── Output dir ───────────────────────────────────
    std::fs::create_dir_all(&args.output)
        .with_context(|| format!("Cannot create output directory: {}", args.output.display()))?;

    Ok(Pipeline {
        strip_gps: args.strip_gps || args.strip_all,
        strip_all: args.strip_all,
        auto_orient: args.auto_orient,
        resize,
        no_upscale: args.no_upscale,
        filter: args.filter.into(),
        rotate: args.rotate,
        flip_h: args.flip_h,
        flip_v: args.flip_v,
        border_px: args.border,
        border_rgba,
        brightness: args.brightness,
        contrast: args.contrast,
        sharpen: args.sharpen,
        grayscale: args.grayscale,
        output_format: args.format,
        quality: args.quality,
        output_dir: args.output.clone(),
        prefix: args.prefix.clone(),
        suffix: args.suffix.clone(),
        overwrite: args.overwrite,
        dry_run: args.dry_run,
    })
}

/// Parse a CSS-style color string into RGBA.
pub fn parse_color(color: &str) -> Result<image::Rgba<u8>> {
    // Named colors
    let rgba = match color.to_lowercase().as_str() {
        "white" => image::Rgba([255, 255, 255, 255]),
        "black" => image::Rgba([0, 0, 0, 255]),
        "red" => image::Rgba([255, 0, 0, 255]),
        "green" => image::Rgba([0, 128, 0, 255]),
        "blue" => image::Rgba([0, 0, 255, 255]),
        "gray" | "grey" => image::Rgba([128, 128, 128, 255]),
        "transparent" => image::Rgba([0, 0, 0, 0]),
        hex if hex.starts_with('#') => {
            let hex = hex.trim_start_matches('#');
            if hex.len() == 6 {
                let r = u8::from_str_radix(&hex[0..2], 16)?;
                let g = u8::from_str_radix(&hex[2..4], 16)?;
                let b = u8::from_str_radix(&hex[4..6], 16)?;
                image::Rgba([r, g, b, 255])
            } else if hex.len() == 8 {
                let r = u8::from_str_radix(&hex[0..2], 16)?;
                let g = u8::from_str_radix(&hex[2..4], 16)?;
                let b = u8::from_str_radix(&hex[4..6], 16)?;
                let a = u8::from_str_radix(&hex[6..8], 16)?;
                image::Rgba([r, g, b, a])
            } else {
                return Err(BatImgError::InvalidColor(color.to_string()).into());
            }
        }
        _ => return Err(BatImgError::InvalidColor(color.to_string()).into()),
    };

    Ok(rgba)
}
