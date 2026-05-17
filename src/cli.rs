use clap::{ArgAction, Parser, ValueEnum};
use std::path::PathBuf;

/// bat_img_rs — fast multithreaded batch image processor
#[derive(Parser, Debug, Clone)]
#[command(
    name = "bat_img_rs",
    version,
    about = "Fast multithreaded batch image processor",
    long_about = None,
    after_help = "\
EXAMPLES:
  # Strip GPS in-place (no --output = overwrite originals)
  bat_img_rs -i ./photos --strip-gps

  # Strip GPS from all JPEGs in a folder, save to ./output
  bat_img_rs -i ./photos/*.jpg --strip-gps -o ./output

  # Resize to max 1920px wide, add a white border, convert to WebP
  bat_img_rs -i ./photos -r 1920x0 --border 10 --border-color white -f webp -o ./out

  # Resize keeping aspect ratio (height-constrained), quality 85, 8 threads
  bat_img_rs -i ./raw -r 0x1080 -q 85 -t 8 -o ./web

  # Rotate 90°, flip horizontal, strip all metadata
  bat_img_rs -i ./scans --rotate 90 --flip-h --strip-all -o ./clean

  # Sharpen + brightness/contrast adjustment
  bat_img_rs -i ./input --sharpen --brightness 10 --contrast 15 -o ./enhanced
  
  # Quiet mode (long flag only; -q is reserved for --quality)
  bat_img_rs -i ./photos --strip-gps -o ./clean --quiet
"
)]
pub struct Args {
    // ── Input / Output ───────────────────────────────────────────────────────

    /// Input: file path, glob pattern, or directory (e.g. ./photos, "*.jpg", ./img/photo.png)
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,

    /// Output directory. When omitted, each input file is processed in-place
    /// (the original is overwritten). A temp file + atomic rename is used so
    /// the original is never corrupted on failure.
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Recurse into subdirectories when input is a directory
    #[arg(short = 'R', long, action = ArgAction::SetTrue)]
    pub recursive: bool,

    // ── Metadata ─────────────────────────────────────────────────────────────

    /// Strip GPS location data from EXIF metadata
    #[arg(long, action = ArgAction::SetTrue)]
    pub strip_gps: bool,

    /// Strip ALL EXIF/IPTC/XMP metadata (implies --strip-gps)
    #[arg(long, action = ArgAction::SetTrue)]
    pub strip_all: bool,

    // ── Resize ───────────────────────────────────────────────────────────────

    /// Resize image. Format: WIDTHxHEIGHT (e.g. 1920x1080).
    /// Use 0 for auto (e.g. 1920x0 = fit width, 0x1080 = fit height).
    #[arg(short, long, value_name = "WxH")]
    pub resize: Option<String>,

    /// Resize filter algorithm
    #[arg(long, value_enum, default_value = "lanczos3")]
    pub filter: FilterType,

    /// Do not upscale images smaller than the target size
    #[arg(long, action = ArgAction::SetTrue)]
    pub no_upscale: bool,

    // ── Border ───────────────────────────────────────────────────────────────

    /// Add a border of N pixels on each side
    #[arg(long, value_name = "PIXELS")]
    pub border: Option<u32>,

    /// Border color as CSS hex (#rrggbb) or name (white, black, red…)
    #[arg(long, default_value = "white", value_name = "COLOR")]
    pub border_color: String,

    // ── Rotation / Flip ──────────────────────────────────────────────────────

    /// Rotate image clockwise by degrees (90, 180, 270)
    #[arg(long, value_name = "DEGREES")]
    pub rotate: Option<u32>,

    /// Flip image horizontally (mirror left-right)
    #[arg(long, action = ArgAction::SetTrue)]
    pub flip_h: bool,

    /// Flip image vertically (mirror top-bottom)
    #[arg(long, action = ArgAction::SetTrue)]
    pub flip_v: bool,

    /// Auto-rotate based on EXIF orientation tag before applying other transforms
    #[arg(long, action = ArgAction::SetTrue)]
    pub auto_orient: bool,

    // ── Color / Adjustments ──────────────────────────────────────────────────

    /// Brightness adjustment (-100 to +100)
    #[arg(long, value_name = "VALUE", allow_negative_numbers = true)]
    pub brightness: Option<i32>,

    /// Contrast adjustment (-100 to +100)
    #[arg(long, value_name = "VALUE", allow_negative_numbers = true)]
    pub contrast: Option<f32>,

    /// Apply sharpening filter
    #[arg(long, action = ArgAction::SetTrue)]
    pub sharpen: bool,

    /// Convert to grayscale
    #[arg(long, action = ArgAction::SetTrue)]
    pub grayscale: bool,

    // ── Output format / Quality ──────────────────────────────────────────────

    /// Output format (defaults to same as input)
    #[arg(short, long, value_enum)]
    pub format: Option<OutputFormat>,

    /// JPEG/WebP output quality (1–100). When omitted, HEIC files re-encode
    /// using the encoder default (closest to original size). Required for
    /// non-HEIC outputs; defaults to 90 if unset.
    #[arg(short = 'q', long, value_name = "1-100")]
    pub quality: Option<u8>,

    /// Filename suffix appended before extension (e.g. "_edited" → photo_edited.jpg)
    #[arg(long, default_value = "", value_name = "SUFFIX")]
    pub suffix: String,

    /// Filename prefix prepended (e.g. "web_" → web_photo.jpg)
    #[arg(long, default_value = "", value_name = "PREFIX")]
    pub prefix: String,

    // ── Processing ───────────────────────────────────────────────────────────

    /// Number of threads to use (default: number of logical CPUs)
    #[arg(short, long, default_value_t = num_cpus())]
    pub threads: usize,

    /// Overwrite existing output files (default: skip)
    #[arg(long, action = ArgAction::SetTrue)]
    pub overwrite: bool,

    /// Suppress all output except errors
    #[arg(long, action = ArgAction::SetTrue)]
    pub quiet: bool,

    /// Dry-run: show what would be done without processing
    #[arg(long, action = ArgAction::SetTrue)]
    pub dry_run: bool,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq)]
pub enum FilterType {
    Nearest,
    Triangle,
    CatmullRom,
    Gaussian,
    Lanczos3,
}

impl From<FilterType> for image::imageops::FilterType {
    fn from(f: FilterType) -> Self {
        match f {
            FilterType::Nearest => image::imageops::FilterType::Nearest,
            FilterType::Triangle => image::imageops::FilterType::Triangle,
            FilterType::CatmullRom => image::imageops::FilterType::CatmullRom,
            FilterType::Gaussian => image::imageops::FilterType::Gaussian,
            FilterType::Lanczos3 => image::imageops::FilterType::Lanczos3,
        }
    }
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq)]
pub enum OutputFormat {
    Jpeg,
    Png,
    Webp,
    Tiff,
    Bmp,
    Gif,
    Heic,
    Heif,
}

impl OutputFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            OutputFormat::Jpeg => "jpg",
            OutputFormat::Png => "png",
            OutputFormat::Webp => "webp",
            OutputFormat::Tiff => "tiff",
            OutputFormat::Bmp => "bmp",
            OutputFormat::Gif => "gif",
            OutputFormat::Heic => "heic",
            OutputFormat::Heif => "heif",
        }
    }
}

pub fn parse() -> Args {
    Args::parse()
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}
