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
```

Pre-built binaries are provided for:

| Platform | Architecture |
|---|---|
| macOS | Apple Silicon (M1 / M2 / M3 / M4) |
| macOS | Intel (x86-64) |
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

# Strip ALL metadata, auto-orient, sharpen — 8 threads, recurse
bat_img -i ./raw -R --strip-all --auto-orient --sharpen -t 8 -o ./export
```

## Features

| Feature | Flag |
|---|---|
| **In-place processing** — overwrite originals | *(omit `--output`)* |
| Strip GPS location from EXIF | `--strip-gps` |
| Strip ALL metadata (EXIF, IPTC, XMP) | `--strip-all` |
| Auto-orient from EXIF | `--auto-orient` |
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
  -i, --input <INPUT>...       File, glob pattern, or directory
  -o, --output <DIR>           Output directory.
                               Omit to process files in-place.
  -R, --recursive              Recurse into subdirectories
  -t, --threads <N>            Thread count [default: CPU core count]
  -q, --quality <1-100>        JPEG/WebP quality (HEIC preserves original
                               quality when omitted)
  -f, --format <FORMAT>        Output format:
                               heic | heif | jpeg | png | webp | tiff | bmp | gif
      --strip-gps              Remove GPS location from EXIF
      --strip-all              Remove all metadata
      --auto-orient            Auto-rotate from EXIF orientation tag
  -r, --resize <WxH>           Resize (use 0 for auto: 1920x0 or 0x1080)
      --filter <FILTER>        Resize filter [default: lanczos3]
                               nearest|triangle|catmull-rom|gaussian|lanczos3
      --no-upscale             Never upscale smaller images
      --border <PIXELS>        Add border N pixels wide on all sides
      --border-color <COLOR>   Border color: name or #rrggbb [default: white]
      --rotate <DEG>           Rotate clockwise: 90 | 180 | 270
      --flip-h                 Flip horizontally
      --flip-v                 Flip vertically
      --brightness <VALUE>     Brightness delta (-100..+100)
      --contrast <VALUE>       Contrast delta (-100..+100)
      --sharpen                Apply unsharp mask
      --grayscale              Convert to grayscale
      --prefix <PREFIX>        Prepend string to output filename
      --suffix <SUFFIX>        Append string to output filename
      --overwrite              Overwrite existing output files
      --dry-run                Preview without writing files
      --quiet                  Suppress output except errors
  -h, --help                   Print help
  -V, --version                Print version
```

### In-place mode

Omitting `--output` overwrites each original file in place. A temp file is
written first and then atomically renamed over the original, so the source
is never corrupted if something goes wrong.

```bash
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
