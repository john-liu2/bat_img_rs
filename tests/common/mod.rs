// Test helpers in tests/common/mod.rs

use image::{DynamicImage, RgbImage};
use tempfile::TempDir;
use std::path::PathBuf;

// ── TIFF / EXIF byte-building helpers ─────────────────────────────────────

#[allow(dead_code)]
pub fn png_with_exif_chunk(exif: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");

    // eXIf chunk
    out.extend_from_slice(&(exif.len() as u32).to_be_bytes());
    out.extend_from_slice(b"eXIf");
    out.extend_from_slice(exif);

    // Fake CRC (not needed by your chunk rewriter)
    out.extend_from_slice(&0u32.to_be_bytes());

    // IEND chunk
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(b"IEND");
    out.extend_from_slice(&0u32.to_be_bytes());
    out
}

#[allow(dead_code)]
pub fn webp_with_exif_chunk(exif: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();

    body.extend_from_slice(b"EXIF");
    body.extend_from_slice(&(exif.len() as u32).to_le_bytes());
    body.extend_from_slice(exif);

    if exif.len() % 2 == 1 {
        body.push(0);
    }
    let file_size = 4 + body.len();
    let mut out = Vec::new();

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&body);
    out
}

/// Build a minimal little-endian TIFF block containing a single IFD with
/// the given tag entries.  Each entry is `(tag, type, value_u16)`.
#[allow(dead_code)]
pub fn build_tiff_le(entries: &[(u16, u16, u16)]) -> Vec<u8> {
    // TIFF header: "II" + magic 42 (LE) + IFD offset = 8
    let mut buf: Vec<u8> = vec![
        b'I', b'I', // little-endian
        0x2A, 0x00, // magic
        0x08, 0x00, 0x00, 0x00, // IFD at offset 8
    ];

    // IFD: entry count (u16 LE)
    let count = entries.len() as u16;
    buf.extend_from_slice(&count.to_le_bytes());

    for &(tag, typ, val) in entries {
        buf.extend_from_slice(&tag.to_le_bytes()); // tag
        buf.extend_from_slice(&typ.to_le_bytes()); // type (3 = SHORT)
        buf.extend_from_slice(&1u32.to_le_bytes()); // count = 1
        // For SHORT values ≤ 4 bytes, the value is stored directly in the
        // value-offset field (little-endian, zero-padded).
        buf.extend_from_slice(&(val as u32).to_le_bytes());
    }

    // Next-IFD offset = 0 (no more IFDs)
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf
}

/// Build a minimal big-endian TIFF block with a single orientation entry.
#[allow(dead_code)]pub fn build_tiff_be(orientation: u16) -> Vec<u8> {
    let mut buf: Vec<u8> = vec![
        b'M', b'M', // big-endian
        0x00, 0x2A, // magic
        0x00, 0x00, 0x00, 0x08, // IFD at offset 8
    ];
    // 1 entry
    buf.extend_from_slice(&1u16.to_be_bytes());
    // tag 0x0112 = Orientation
    buf.extend_from_slice(&0x0112u16.to_be_bytes());
    buf.extend_from_slice(&3u16.to_be_bytes()); // SHORT
    buf.extend_from_slice(&1u32.to_be_bytes()); // count
    // TIFF spec: for a SHORT stored inline in the 4-byte value-offset field,
    // big-endian layout puts the value in the *first* 2 bytes of those 4 bytes.
    buf.extend_from_slice(&orientation.to_be_bytes()); // value (2 bytes)
    buf.extend_from_slice(&[0x00, 0x00]); // padding (2 bytes)
    buf.extend_from_slice(&0u32.to_be_bytes()); // next IFD
    buf
}

/// Wrap a TIFF block in a minimal JPEG APP1 EXIF segment.
#[allow(dead_code)]pub fn jpeg_with_exif(tiff: &[u8]) -> Vec<u8> {
    let exif_header = b"Exif\x00\x00";
    let payload_len = exif_header.len() + tiff.len();
    let seg_len = (payload_len + 2) as u16; // includes the length field

    let mut jpeg = vec![0xFF, 0xD8]; // SOI
    jpeg.push(0xFF);
    jpeg.push(0xE1); // APP1
    jpeg.extend_from_slice(&seg_len.to_be_bytes());
    jpeg.extend_from_slice(exif_header);
    jpeg.extend_from_slice(tiff);
    // Append a minimal SOS marker so the file looks structurally complete
    jpeg.extend_from_slice(&[0xFF, 0xD9]); // EOI
    jpeg
}

/// Build a TIFF with a GPS IFD pointer (tag 0x8825) set to a non-zero offset.
#[allow(dead_code)]
pub fn build_tiff_with_gps(gps_offset: u32) -> Vec<u8> {
    let mut buf: Vec<u8> = vec![b'I', b'I', 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00];
    // 2 entries: Orientation + GPSInfoIFD
    buf.extend_from_slice(&2u16.to_le_bytes());

    // Orientation = 1
    buf.extend_from_slice(&0x0112u16.to_le_bytes());
    buf.extend_from_slice(&3u16.to_le_bytes());
    buf.extend_from_slice(&1u32.to_le_bytes());
    buf.extend_from_slice(&1u32.to_le_bytes());

    // GPSInfoIFDPointer = gps_offset
    buf.extend_from_slice(&0x8825u16.to_le_bytes());
    buf.extend_from_slice(&4u16.to_le_bytes()); // LONG
    buf.extend_from_slice(&1u32.to_le_bytes());
    buf.extend_from_slice(&gps_offset.to_le_bytes());

    buf.extend_from_slice(&0u32.to_le_bytes()); // next IFD
    buf
}

// ── test_processor.rs Helpers ───────────────────────────────────────────────────────────────

/// Solid-colour RGB test image.
#[allow(dead_code)]
pub fn solid_rgb(w: u32, h: u32, r: u8, g: u8, b: u8) -> DynamicImage {
    let mut img = RgbImage::new(w, h);
    for pixel in img.pixels_mut() {
        *pixel = image::Rgb([r, g, b]);
    }
    DynamicImage::ImageRgb8(img)
}

/// Save an image as JPEG and return its path.
#[allow(dead_code)]
pub fn save_jpeg(img: &DynamicImage, dir: &TempDir, name: &str) -> PathBuf {
    let path = dir.path().join(name);
    let rgb = img.to_rgb8();
    let mut f = std::fs::File::create(&path).unwrap();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut f, 95);
    enc.encode(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        image::ExtendedColorType::Rgb8,
    )
    .unwrap();
    path
}

/// Save an image as PNG and return its path.
#[allow(dead_code)]
pub fn save_png(img: &DynamicImage, dir: &TempDir, name: &str) -> PathBuf {
    let path = dir.path().join(name);
    img.save(&path).unwrap();
    path
}
