# bat_img_rs

A fast, **multithreaded** batch image processor written in Rust.

## Features

| Feature | Flag |
|---|---|
| In-place processing (overwrite originals) | *(omit `--output`)* |
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
| Format conversion (incl. HEIC/HEIF) | `-f heic / webp / png / jpeg / tiff / bmp` |
| JPEG/WebP quality | `-q 85` |
| Filename prefix/suffix | `--prefix web_ --suffix _sm` |
| Parallel threads | `-t 8` |
| Dry-run preview | `--dry-run` |
| Overwrite existing output files | `--overwrite` |
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

> **HEIC encoding**
> Requires libheif built with x265 (HEVC encoder) support — Homebrew's
> `libheif` includes this by default. If encoding fails with
> *"no HEVC encoder available"*, run `brew reinstall libheif`.

## Usage

```
bat_img_rs [OPTIONS] --input <INPUT>...
```

### Options

```
  -i, --input <INPUT>...       File, glob, or directory
  -o, --output <DIR>           Output directory. When omitted, files are
                               processed in-place (originals overwritten)
  -R, --recursive              Recurse into subdirectories
  -t, --threads <N>            Thread count [default: # of CPU cores]
  -q, --quality <1-100>        JPEG/WebP quality. When omitted for HEIC,
                               the encoder default is used to preserve
                               original file size
  -f, --format <FORMAT>        Output format:
                               heic | heif | jpeg | png | webp | tiff | bmp | gif
      --strip-gps              Remove GPS location from EXIF
      --strip-all              Remove all metadata (EXIF, IPTC, XMP)
      --auto-orient            Auto-rotate per EXIF orientation tag
  -r, --resize <WxH>           Resize image (0 = auto-scale, e.g. 1920x0)
      --filter <FILTER>        Resize filter [default: lanczos3]
                               nearest | triangle | catmull-rom | gaussian | lanczos3
      --no-upscale             Never upscale images smaller than the target
      --border <PIXELS>        Add a solid border N pixels wide on each side
      --border-color <COLOR>   Border color: name or #rrggbb [default: white]
      --rotate <DEG>           Rotate clockwise: 90 | 180 | 270
      --flip-h                 Flip horizontally (mirror left-right)
      --flip-v                 Flip vertically (mirror top-bottom)
      --brightness <VALUE>     Brightness delta (-100..+100)
      --contrast <VALUE>       Contrast delta (-100.0..+100.0)
      --sharpen                Apply unsharp mask
      --grayscale              Convert to grayscale
      --prefix <PREFIX>        Prepend string to output filenames
      --suffix <SUFFIX>        Append string to output filenames (before ext)
      --overwrite              Overwrite existing files in the output directory
      --dry-run                Preview what would be done without processing
      --quiet                  Suppress all output except errors
  -h, --help                   Print help
  -V, --version                Print version
```

### In-place mode

When `--output` is omitted, bat_img_rs processes each file **in-place**: the
original is overwritten with the processed result. A sibling temp file is
written first and then atomically renamed over the original, so the source
is never corrupted if encoding fails mid-write.

**Constraints in in-place mode:**

- `--format` cannot change the file extension (e.g. converting `.jpg → .webp`
  in-place would silently rename the file). Specify `--output` when changing
  formats.
- `--prefix` and `--suffix` have no effect on the output filename since the
  original path is reused.

## Examples

### In-place processing

```bash
# Strip GPS from all iPhone photos — originals overwritten
bat_img_rs -i ~/Pictures/iPhone --strip-gps

# Strip ALL metadata from every image recursively
bat_img_rs -i ./archive -R --strip-all

# Resize all HEICs to 2048px wide, keep HEIC format
bat_img_rs -i ./photos -r 2048x0

# Auto-orient, sharpen, and strip GPS — all in one pass, 8 threads
bat_img_rs -i ./raw -R --auto-orient --sharpen --strip-gps -t 8
```

### Output to a directory

```bash
# Strip GPS and save to ./clean  (originals untouched)
bat_img_rs -i ./photos --strip-gps -o ./clean

# Resize to 1920px wide, add 10px white border, convert to WebP at quality 85
bat_img_rs -i ./photos -r 1920x0 --border 10 --border-color white -f webp -q 85 -o ./web

# Convert HEIC → JPEG at quality 90
bat_img_rs -i ./iphone_photos/*.heic -f jpeg -q 90 -o ./jpegs

# Convert HEIC → WebP at quality 85, resize to 2048px wide
bat_img_rs -i ./iphone_photos -r 2048x0 -f webp -q 85 -o ./web

# Rotate 90°, flip horizontal, convert to grayscale, add _bw suffix
bat_img_rs -i ./scans --rotate 90 --flip-h --grayscale --suffix _bw -o ./processed

# Brightness +10, contrast +15, prefix "web_"
bat_img_rs -i ./input --brightness 10 --contrast 15 --prefix web_ -o ./enhanced

# Strip all metadata, auto-orient, sharpen, 8 threads, recurse
bat_img_rs -i ./raw -R --strip-all --auto-orient --sharpen -t 8 -o ./export

# Resize with height constraint (fit to 1080px tall, keep aspect ratio)
bat_img_rs -i ./landscape/*.jpg -r 0x1080 --filter lanczos3 -o ./resized
```

### Dry-run

```bash
# Preview what would happen without writing any files
bat_img_rs -i ./photos -r 800x600 --strip-gps --dry-run
```

### Output mode summary

| Command | Behaviour |
|---|---|
| `bat_img_rs -i ./photos --strip-gps` | In-place: originals overwritten |
| `bat_img_rs -i ./photos --strip-gps -o ./out` | Output to `./out/`: originals untouched |
| `bat_img_rs -i ./photos -f webp -o ./out` | Convert to WebP in `./out/` |
| `bat_img_rs -i ./photos -f webp` | Error: format change requires `--output` |

## Architecture

```
src/
├── main.rs        — Entry point: parses args, collects files, drives Rayon parallel loop
├── cli.rs         — Clap-derive CLI definition (all flags and their types)
├── pipeline.rs    — File collection (glob/dir/recursive) + Pipeline struct (validated config)
├── processor.rs   — ProcessingContext: runs the full pipeline on one image
├── heic.rs        — HEIC/HEIF decode + encode via libheif-rs; preserves source codec
├── exif.rs        — JPEG byte-level EXIF parser: strip GPS IFD, strip all APP segments,
│                    read orientation tag for auto-orient
└── error.rs       — Custom error types (thiserror)
```

### Concurrency model

```
main thread
  └─ collects Vec<PathBuf>
  └─ builds Arc<Pipeline>   (validated, immutable, shared across all threads)
  └─ rayon::par_iter()
       ├─ thread 1 → ProcessingContext::process(file_a)
       ├─ thread 2 → ProcessingContext::process(file_b)
       ├─ thread 3 → ProcessingContext::process(file_c)
       └─ … (N threads from --threads flag)
```

Each thread:
1. Reads the file (raw bytes for JPEG/PNG/etc., or via libheif for HEIC)
2. Optionally strips/rewrites EXIF at the byte level before pixel decode
3. Decodes image pixels into a `DynamicImage`
4. Applies transforms in order: orient → grayscale → resize → brightness/contrast → sharpen → rotate → flip → border
5. Encodes to a temp file, then atomically renames it to the final path (in-place or output dir)

There are **no shared mutable data structures** — the pipeline config is read-only and image buffers are per-thread, so the tool scales linearly with core count.

## License

MIT
