# bat_img

**Fast, multithreaded batch image processor** — available as a standalone
command-line tool installable via `pip`.

The binary is a native [Rust](https://www.rust-lang.org/) executable
(no Python overhead at runtime). The Python package is simply a convenient
delivery mechanism so you can install bat_img the same way you install
any other command-line tool.

## Installation

```bash
pip install bat_img

brew install libheif  # for HEIC/HEIF support
```

Pre-built binaries are provided for:

| Platform | Architecture |
|---|---|
| macOS | Apple Silicon |
| Linux | x86-64 (glibc 2.17+, compatible with most distros) |
| Windows | x86-64 |

After installation, the `bat_img` command is available in your terminal.

## Quick start

```bash
# Strip GPS location from all iPhone photos (in-place)
bat_img -i ~/Pictures/iPhone --strip-gps

# Resize all JPEGs to 1920 px wide, save to ./web/
bat_img -i ./photos -r 1920x0 -o ./web

# Convert HEIC → WebP at quality 85, resize to 2048 px wide
bat_img -i ./iphone_photos -r 2048x0 -f webp -q 85 -o ./web

# Strip ALL metadata, sharpen — 8 threads, recurse
bat_img -i ./raw -R --strip-all --sharpen -t 8 -o ./export
```

## Features

| Feature | Flag |
|---|---|
| **In-place processing** — overwrite originals | *(omit `--output`)* |
| Print image metadata (dimensions, size, format, date/time, etc.) | `--info` |
| Strip GPS location from EXIF | `--strip-gps` |
| Strip ALL metadata (EXIF, IPTC, XMP) | `--strip-all` |
| Resize (width, height, or both) | `-r 1920x0` |
| No-upscale guard | `--no-upscale` |
| Resize filter | `--filter lanczos3` |
| Add solid border | `--border 20 --border-color "#fff"` |
| Rotate 90 / 180 / 270° | `--rotate 90` |
| Flip horizontal / vertical | `--flip-h` / `--flip-v` |
| Brightness adjustment | `--brightness 10` |
| Contrast adjustment | `--contrast 15` |
| Sharpen | `--sharpen` |
| Grayscale | `--grayscale` |
| Format conversion incl. HEIC | `-f heic / webp / png / jpeg / tiff` |
| JPEG / WebP quality | `-q 85` |
| Filename prefix / suffix | `--prefix web_ --suffix _sm` |
| Parallel threads | `-t 8` |
| Dry-run preview | `--dry-run` |
| Recursive directory walk | `-R` |

---

## Usage

```
bat_img [OPTIONS] --input <INPUT>...
```

### All options

```
  -i, --input <INPUT>...      Input: file path, glob pattern, or directory (e.g. ./photos, "*.jpg", ./img/photo.png)
  -o, --output <OUTPUT>       Output directory. When omitted, each input file is processed in-place (the original is overwritten)
  -R, --recursive             Recurse into subdirectories when input is a directory
      --info                  Print image metadata (dimensions, size, format, date/time, etc.)
      --strip-gps             Strip GPS location data from EXIF metadata
      --strip-all             Strip ALL EXIF/IPTC/XMP metadata (implies --strip-gps)
  -r, --resize <WxH>          Resize image. Format: WIDTHxHEIGHT (e.g. 1920x1080). Use 0 for auto (e.g. 1920x0 = fit width, 0x1080 = fit height)
      --filter <FILTER>       Resize filter algorithm [default: lanczos3] [possible values: nearest, triangle, catmull-rom, gaussian, lanczos3]
      --no-upscale            Do not upscale images smaller than the target size
      --border <PIXELS>       Add a border of N pixels on each side
      --border-color <COLOR>  Border color as CSS hex (#rrggbb) or name (white, black, red…) [default: white]
      --rotate <DEGREES>      Rotate image clockwise by degrees (90, 180, 270)
      --flip-h                Flip image horizontally (mirror left-right)
      --flip-v                Flip image vertically (mirror top-bottom)
      --brightness <VALUE>    Brightness adjustment (-100 to +100)
      --contrast <VALUE>      Contrast adjustment (-100 to +100)
      --sharpen               Apply sharpening filter
      --grayscale             Convert to grayscale
  -f, --format <FORMAT>       Output format (defaults to same as input) [possible values: jpeg, png, webp, tiff, bmp, gif, heic, heif]
  -q, --quality <1-100>       JPEG/WebP output quality (1–100), required for non-HEIC output. Default is 90 if not set. HEIC file is encoded with the default encoder
      --suffix <SUFFIX>       Filename suffix appended before extension (e.g. "_edited" → photo_edited.jpg) [default: ""]
      --prefix <PREFIX>       Filename prefix prepended (e.g. "web_" → web_photo.jpg) [default: ""]
  -t, --threads <THREADS>     Number of threads to use (default: physical CPUs qty on macOS; logical CPUs qty on others) [default: 8]
      --overwrite             Overwrite existing output files (default: skip)
      --quiet                 Suppress all output except errors
      --dry-run               Dry-run: show what would be done without processing
  -h, --help                  Print help
  -V, --version               Print version
```

### In-place mode

Omitting `--output` overwrites each original file in place. A temp file is
written first and then atomically renamed over the original, so the source
is never corrupted if something goes wrong.

```bash
# Show all image files meta data
bat_img -i ./photos --info

# Strip GPS from every HEIC file recursively — no copies made
bat_img -i ~/Pictures -R --strip-gps

# Resize all JPEGs to 2048 px wide, in-place
bat_img -i ./photos -r 2048x0
```

**Note:** in-place mode cannot change the file format (e.g. HEIC → WebP).
Use `--output` when changing formats.

### Examples

```bash
# Add a 20 px black border to all PNGs
bat_img -i ./screenshots --border 20 --border-color black -o ./bordered

# Rotate scans 90° clockwise and convert to grayscale
bat_img -i ./scans --rotate 90 --grayscale -o ./processed

# Convert HEIC → JPEG at quality 90, resize to fit 1920×1080
bat_img -i ./iphone_photos -f jpeg -q 90 -r 1920x1080 -o ./jpegs

# Dry-run — see what would happen without writing anything
bat_img -i ./photos -r 800x600 --strip-gps --dry-run
```

## HEIC support

bat_img can read and write HEIC/HEIF files natively, including:

- Decoding HEIC photos from iPhone / iPad
- Re-encoding back to HEIC while preserving the original codec (HEVC / AV1)
  and file size (unless `--quality` is specified)
- Converting HEIC to any other supported format with `-f jpeg`, `-f webp`, etc.

## License

**bat_img** is distributed under MIT License. Please see details in
[LICENSE](https://github.com/john-liu2/bat_img_rs/blob/main/LICENSE).
