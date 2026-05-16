# bat_img_rs

A fast, **multithreaded** batch image processor written in Rust.

## Features

| Feature | Flag |
|---|---|
| Strip GPS location from EXIF | `--strip-gps` |
| Strip ALL metadata | `--strip-all` |
| Auto-orient from EXIF | `--auto-orient` |
| Resize (width, height, or both) | `-r 1920x0` |
| No-upscale guard | `--no-upscale` |
| Resize filter (nearest/lanczos3/…) | `--filter lanczos3` |
| Add border (px + color) | `--border 20 --border-color "#fff"` |
| Rotate 90/180/270° | `--rotate 90` |
| Flip horizontal / vertical | `--flip-h` / `--flip-v` |
| Brightness adjustment | `--brightness 10` |
| Contrast adjustment | `--contrast 15.0` |
| Sharpen | `--sharpen` |
| Grayscale conversion | `--grayscale` |
| Format conversion | `-f webp / png / jpeg / tiff / bmp` |
| JPEG/WebP quality | `-q 85` |
| Filename prefix/suffix | `--prefix web_ --suffix _sm` |
| Parallel threads | `-t 8` |
| Dry-run preview | `--dry-run` |
| Overwrite existing | `--overwrite` |
| Recursive directory walk | `-R` |

## Requirements

- macOS 12+ (Intel or Apple Silicon)
- [Rust toolchain](https://rustup.rs) (stable, 1.78+)
- **libheif** (for HEIC/HEIF support):

```bash
brew install libheif        # pulls in libde265 + libaom automatically
```

## Build

```bash
# Install Rust if not already installed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install libheif (macOS)
brew install libheif

# Clone / unzip this project, then:
cd bat_img_rs
cargo build --release

# Binary will be at:
./target/release/bat_img_rs

# Optional: install globally
cargo install --path .
```

> **Troubleshooting libheif linkage**
> If `cargo build` fails with *"library not found for -lheif"*, make sure
> `pkg-config` can see it:
> ```bash
> export PKG_CONFIG_PATH="$(brew --prefix libheif)/lib/pkgconfig:$PKG_CONFIG_PATH"
> cargo build --release
> ```

## Usage

```
bat_img_rs [OPTIONS] --input <INPUT>...
```

### Options

```
  -i, --input <INPUT>...       File, glob, or directory
  -o, --output <DIR>           Output directory [default: ./bat_img_rs_out]
  -R, --recursive              Recurse into subdirectories
  -t, --threads <N>            Thread count [default: # of CPU cores]
  -q, --quality <1-100>        JPEG/WebP quality [default: 90]
  -f, --format <FORMAT>        Output format: jpeg|png|webp|tiff|bmp|gif
      --strip-gps              Remove GPS location from EXIF
      --strip-all              Remove all metadata
      --auto-orient            Auto-rotate per EXIF orientation
  -r, --resize <WxH>           Resize (0 = auto, e.g. 1920x0)
      --filter <FILTER>        Resize filter [default: lanczos3]
      --no-upscale             Never upscale smaller images
      --border <PIXELS>        Add border (pixels per side)
      --border-color <COLOR>   Border color: name or #rrggbb [default: white]
      --rotate <DEG>           Rotate clockwise: 90|180|270
      --flip-h                 Flip horizontally
      --flip-v                 Flip vertically
      --brightness <VALUE>     Brightness delta (-100..+100)
      --contrast <VALUE>       Contrast delta (-100.0..+100.0)
      --sharpen                Apply unsharp mask
      --grayscale              Convert to grayscale
      --prefix <PREFIX>        Output filename prefix
      --suffix <SUFFIX>        Output filename suffix
      --overwrite              Overwrite existing output files
      --dry-run                Preview without processing
  -q, --quiet                  Suppress output except errors
  -h, --help                   Print help
  -V, --version                Print version
```

## Examples

```bash
# Strip GPS from all JPEGs in a folder, save to ./clean
bat_img_rs -i ./photos --strip-gps -o ./clean

# Convert iPhone HEIC photos → JPEG at quality 90 (default output for HEIC)
bat_img_rs -i ./iphone_photos/*.heic -o ./jpegs

# Convert HEIC → WebP at quality 85, resize to 2048px wide
bat_img_rs -i ./iphone_photos -r 2048x0 -f webp -q 85 -o ./web

# Strip GPS from HEIC batch, convert to PNG, auto-orient
bat_img_rs -i ~/Photos --strip-gps --auto-orient -f png -o ./clean

# Resize to max 1920px wide, add 10px white border, save as WebP at quality 85
bat_img_rs -i ./photos -r 1920x0 --border 10 --border-color white -f webp -q 85 -o ./web

# Strip ALL metadata, auto-orient, sharpen, 8 threads
bat_img_rs -i ./raw -R --strip-all --auto-orient --sharpen -t 8 -o ./export

# Rotate 90°, flip horizontal, grayscale, add _bw suffix
bat_img_rs -i ./scans --rotate 90 --flip-h --grayscale --suffix _bw -o ./processed

# Brightness +10, contrast +15, prefix "web_"
bat_img_rs -i ./input --brightness 10 --contrast 15 --prefix web_ -o ./enhanced

# Dry-run to preview what would be done
bat_img_rs -i ./photos -r 800x600 --strip-gps --dry-run

# Resize with height constraint (fit to 1080px tall)
bat_img_rs -i ./landscape/*.jpg -r 0x1080 --filter lanczos3 -o ./resized
```

## Architecture

```
src/
├── main.rs        — Entry point: parses args, collects files, drives Rayon parallel loop
├── cli.rs         — Clap-derive CLI definition (all flags and their types)
├── pipeline.rs    — File collection (glob/dir/recursive) + Pipeline struct (validated config)
├── processor.rs   — ProcessingContext: runs the full pipeline on one image
├── heic.rs        — HEIC/HEIF decoder via libheif-rs; returns DynamicImage + raw EXIF bytes
├── exif.rs        — JPEG byte-level EXIF parser: strip GPS IFD, strip all APP segments,
│                    read orientation tag for auto-orient
└── error.rs       — Custom error types (thiserror)
```

### Concurrency model

```
main thread
  └─ collects Vec<PathBuf>
  └─ builds Arc<Pipeline>   (validated, immutable, shared)
  └─ rayon::par_iter()
       ├─ thread 1 → ProcessingContext::process(file_a)
       ├─ thread 2 → ProcessingContext::process(file_b)
       ├─ thread 3 → ProcessingContext::process(file_c)
       └─ … (N threads from --threads flag)
```

Each thread:
1. Reads the file (raw bytes)
2. Optionally processes EXIF in-place at the byte level (no lock needed — each thread has its own buffer)
3. Decodes image pixels
4. Applies transforms in order: orient → grayscale → resize → brightness/contrast → sharpen → rotate → flip → border
5. Encodes and writes to the output directory

There are **no shared mutable data structures** — the pipeline config is read-only and image buffers are per-thread, so the tool scales linearly with core count.

## License

MIT
