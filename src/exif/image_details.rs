//! Compute detailed image properties from raw bytes.

use crate::exif::container::{is_jpeg, is_tiff, is_webp};
use crate::exif::icc::{exif_color_profile_name, get_icc_profile_name};
use image::ColorType;

#[derive(Debug, Default)]
pub struct ImageDetails {
    pub bit_depth: String,
    pub has_alpha: bool,
    pub colorspace: String,
    pub c_profile: String,
    pub chroma_format: Option<String>,
}

pub fn get_image_details(color: ColorType, format: &str, bytes: &[u8]) -> ImageDetails {
    let bits_per_pixel = color.bits_per_pixel();
    let channels = color.channel_count();
    let bit_depth = if channels > 0 {
        format!("{} bits/channel", bits_per_pixel / (channels as u16))
    } else {
        "Unknown".to_string()
    };

    let has_alpha = matches!(color, ColorType::Rgba8 | ColorType::Rgba16);

    let colorspace = if format == "HEIC" {
        if has_alpha { "YCbCr + Alpha" } else { "YCbCr" }
    } else {
        match color {
            ColorType::Rgb8 | ColorType::Rgb16 => {
                if format == "JPEG" || format == "JPG" {
                    "YCbCr"
                } else {
                    "RGB"
                }
            }
            ColorType::Rgba8 | ColorType::Rgba16 => {
                if format == "JPEG" || format == "JPG" {
                    "YCbCr + Alpha"
                } else {
                    "RGBA"
                }
            }
            ColorType::L8 | ColorType::L16 => "Grayscale",
            _ => "Unknown",
        }
    }
    .to_string();

    let mut c_profile =
        get_icc_profile_name(format, bytes).unwrap_or_else(|| "Unknown".to_string());
    if c_profile == "Unknown"
        && let Some(name) = exif_color_profile_name(format, bytes)
    {
        c_profile = name;
    }
    if c_profile == "Unknown" {
        c_profile = "sRGB".to_string();
    }

    // PNG files do not use chroma subsampling (like 4:2:2 or 4:2:0)
    // PNG store full-color for each pixel, equivalent to 4:4:4 (RGB) or 4:4:4:4 (RGBA)
    let png = if has_alpha {
        Some("4:4:4:4".to_string())
    } else {
        Some("4:4:4".to_string())
    };
    let chroma_format = match format {
        "PNG" => png,
        "JPEG" => jpeg_chroma_subsampling(bytes).map(|s| s.to_string()),
        "WEBP" => webp_chroma_subsampling(bytes).map(|s| s.to_string()),
        "TIFF" => tiff_chroma_subsampling(bytes).map(|s| s.to_string()),
        "HEIC" => heic_chroma_format(bytes).map(|s| s.to_string()),
        _ => None,
    };

    ImageDetails {
        bit_depth,
        has_alpha,
        colorspace,
        c_profile,
        chroma_format,
    }
}

// ---- chroma subsampling detection -----------------------------------------

fn jpeg_chroma_subsampling(bytes: &[u8]) -> Option<&'static str> {
    if !is_jpeg(bytes) {
        return None;
    }
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            break;
        }
        let marker = bytes[i + 1];
        if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        if i + 3 >= bytes.len() {
            break;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if len < 2 {
            break;
        }
        if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC {
            let payload_start = i + 4;
            let payload_end = i + 2 + len;
            if payload_end <= bytes.len() && payload_end > payload_start {
                let payload = &bytes[payload_start..payload_end];
                if payload.len() >= 9 {
                    let num_components = payload[5] as usize;
                    if num_components > 0 {
                        let sampling = payload[7];
                        let h = (sampling >> 4) & 0x0F;
                        let v = sampling & 0x0F;
                        return match (h, v) {
                            (1, 1) => Some("4:4:4"),
                            (2, 1) => Some("4:2:2"),
                            (2, 2) => Some("4:2:0"),
                            (4, 1) => Some("4:1:1"),
                            (1, 2) => Some("4:4:0"),
                            _ => None,
                        };
                    }
                }
            }
        }
        i += 2 + len;
    }
    None
}

fn webp_chroma_subsampling(bytes: &[u8]) -> Option<&'static str> {
    if !is_webp(bytes) {
        return None;
    }
    let file_size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let end = (8 + file_size).min(bytes.len());
    let mut pos = 12;
    while pos + 8 <= end {
        let fourcc: [u8; 4] = bytes[pos..pos + 4].try_into().ok()?;
        let size = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        let payload_start = pos + 8;
        let payload_end = payload_start + size;
        if payload_end > bytes.len() {
            break;
        }
        match &fourcc {
            b"VP8 " => {
                let payload = &bytes[payload_start..payload_end];
                if payload.len() >= 3 {
                    let tag = u32::from_le_bytes([payload[0], payload[1], payload[2], 0]);
                    let chroma_bits = (tag >> 6) & 0x03;
                    return match chroma_bits {
                        0 => Some("4:2:0"),
                        1 => Some("4:2:2"),
                        2 => Some("4:4:4"),
                        _ => None,
                    };
                }
                return None;
            }
            b"VP8L" => return Some("4:4:4"),
            _ => {}
        }
        pos = payload_end;
        if !size.is_multiple_of(2) {
            pos += 1;
        }
    }
    None
}

fn tiff_chroma_subsampling(bytes: &[u8]) -> Option<&'static str> {
    if !is_tiff(bytes) {
        return None;
    }
    // if TIFF contains a JPEG stream, use JPEG parser
    if bytes.windows(2).any(|w| w == [0xFF, 0xD8])
        && let Some(jpeg) = jpeg_chroma_subsampling(bytes)
    {
        return Some(jpeg);
    }
    Some("4:4:4")
}

// ---- HEIC chroma helpers (moved from original) ----------------------------

/// Determine HEIC chroma format by scanning for hvcC or av1C boxes.
fn heic_chroma_format(bytes: &[u8]) -> Option<&'static str> {
    hevc_chroma_format(bytes).or_else(|| av1_chroma_format(bytes))
}

fn hevc_chroma_format(bytes: &[u8]) -> Option<&'static str> {
    let payload = find_box_payload_by_type(bytes, b"hvcC")?;
    if payload.len() >= 18 {
        let chroma_format = payload[16] & 0x03;
        return match chroma_format {
            0 => Some("4:0:0"),
            1 => Some("4:2:0"),
            2 => Some("4:2:2"),
            3 => Some("4:4:4"),
            _ => None,
        };
    }
    None
}

fn av1_chroma_format(bytes: &[u8]) -> Option<&'static str> {
    let payload = find_box_payload_by_type(bytes, b"av1C")?;
    if payload.len() >= 3 {
        let subsampling_x = (payload[2] >> 3) & 0x01;
        let subsampling_y = (payload[2] >> 2) & 0x01;
        let mono = (payload[2] >> 4) & 0x01;
        return match (mono, subsampling_x, subsampling_y) {
            (1, _, _) => Some("4:0:0"),
            (0, 0, 0) => Some("4:4:4"),
            (0, 1, 0) => Some("4:2:2"),
            (0, 1, 1) => Some("4:2:0"),
            _ => None,
        };
    }
    None
}

fn find_box_payload_by_type<'a>(bytes: &'a [u8], target: &[u8; 4]) -> Option<&'a [u8]> {
    let mut pos = 0;
    while pos + 8 <= bytes.len() {
        if &bytes[pos..pos + 4] == target {
            if pos < 4 {
                return None;
            }
            let size = u32::from_be_bytes([
                bytes[pos - 4],
                bytes[pos - 3],
                bytes[pos - 2],
                bytes[pos - 1],
            ]) as usize;
            if size >= 8 && pos - 4 + size <= bytes.len() {
                let payload_start = pos + 4;
                let payload_end = pos - 4 + size;
                return Some(&bytes[payload_start..payload_end]);
            }
        }
        pos += 1;
    }
    None
}
