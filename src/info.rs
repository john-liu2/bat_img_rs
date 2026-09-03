// info.rs - read image files info
use crate::exif;
use crate::heic;

use colored::Colorize;
use image::{ColorType, DynamicImage};
use std::path::Path;

const TAB_WIDTH: usize = 15;

/// Convert bytes to human-readable KB, MB, or GB
pub fn easy_file_sz(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < units.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }
    let unit = units[unit_index];

    match unit {
        "B" | "KB" => format!("{} {}", size.round() as u64, unit),
        _ => format!("{:.1} {}", size, unit),
    }
}

/// Format file system metadata (size, last modified) into a string block.
pub fn format_file_metadata(file: &Path) -> String {
    let width = TAB_WIDTH;
    let mut out = String::new();

    if let Ok(metadata) = std::fs::metadata(file) {
        let bytes = metadata.len();
        let readable_size = easy_file_sz(bytes);
        out.push_str(&format!(
            "  {:<width$} : {} ({})\n",
            "File Size".bold(),
            readable_size,
            format!("{} bytes", bytes).dimmed()
        ));
        if let Ok(modified) = metadata.modified() {
            let datetime: chrono::DateTime<chrono::Local> = modified.into();
            out.push_str(&format!(
                "  {:<width$} : {}\n",
                "Last Modified".bold(),
                datetime.format("%Y-%m-%d %H:%M")
            ));
        }
    }
    out
}

/// Format the detected image format into a string line.
pub fn format_format(file: &Path, is_heic: bool) -> String {
    let width = TAB_WIDTH;
    let format_str = if is_heic {
        "HEIF".to_string()
    } else {
        image::ImageFormat::from_path(file)
            .map(|f| format!("{:?}", f))
            .unwrap_or_else(|_| "Unknown".to_string())
    };
    format!(
        "  {:<width$} : {}\n",
        "Format".bold(),
        format_str.to_uppercase().blue()
    )
}

/// Format image dimensions (width x height, megapixels).
pub fn format_dimensions(img: &DynamicImage) -> String {
    let width = TAB_WIDTH;
    let (w, h) = (img.width(), img.height());
    let megapixels = (w as f64 * h as f64) / 1_000_000.0;
    format!(
        "  {:<width$} : {} x {} ({:.1} MP)\n",
        "Dimensions".bold(),
        w,
        h,
        megapixels
    )
}

/// Format technical image details (bit depth, alpha, color space, etc.).
pub fn format_image_details(details: &exif::ImageDetails) -> String {
    let width = TAB_WIDTH;
    let mut out = String::new();
    out.push_str(&format!(
        "  {:<width$} : {}\n",
        "Bit Depth".bold(),
        details.bit_depth
    ));
    out.push_str(&format!(
        "  {:<width$} : {}\n",
        "Alpha Channel".bold(),
        if details.has_alpha { "Yes" } else { "No" }
    ));
    out.push_str(&format!(
        "  {:<width$} : {}\n",
        "Color Space".bold(),
        details.colorspace
    ));
    out.push_str(&format!(
        "  {:<width$} : {}\n",
        "Color Profile".bold(),
        details.c_profile
    ));
    if let Some(chroma) = &details.chroma_format {
        out.push_str(&format!(
            "  {:<width$} : {}\n",
            "Chroma Format".bold(),
            chroma
        ));
    }
    out
}

/// Format EXIF camera metadata into a string block.
pub fn format_exif_metadata(file: &Path) -> String {
    let wid2 = TAB_WIDTH - 1;
    let mut out = String::new();
    out.push_str("\n  ");
    out.push_str(&"[ EXIF Metadata ]".bold().underline().dimmed().to_string());
    out.push('\n');

    if let Some(exif_data) = exif::read_exif(file) {
        let mut found_any = false;

        if let Some(ref make) = exif_data.make {
            out.push_str(&format!("    {:<wid2$} : {}\n", "Make", make));
            found_any = true;
        }
        if let Some(ref model) = exif_data.model {
            out.push_str(&format!("    {:<wid2$} : {}\n", "Model", model));
            found_any = true;
        }
        if let Some(ref date) = exif_data.date_time {
            // Convert strings like "2025-12-20 20:02:28" to "2025-12-20 20:02"
            out.push_str(&format!("    {:<wid2$} : {}\n", "Date/Time", &date[..16]));
            found_any = true;
        }
        if let Some(ref iso) = exif_data.iso {
            out.push_str(&format!("    {:<wid2$} : ISO {}\n", "ISO Speed", iso));
            found_any = true;
        }
        if let Some(ref exp) = exif_data.exposure {
            let clean_exp = exp.trim().trim_end_matches('s').trim();
            out.push_str(&format!("    {:<wid2$} : {} s\n", "Exposure", clean_exp));
            found_any = true;
        }
        if let Some(ref f) = exif_data.f_number {
            let clean_f = f
                .trim()
                .trim_start_matches("f/")
                .trim_start_matches('f')
                .trim();
            let formatted_f = if let Ok(val) = clean_f.parse::<f64>() {
                format!("{:.2}", val)
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_string()
            } else {
                clean_f.to_string()
            };
            out.push_str(&format!("    {:<wid2$} : f/{}\n", "Aperture", formatted_f));
            found_any = true;
        }
        if let Some(ref fl) = exif_data.focal_length {
            let clean_fl = fl.trim().trim_end_matches("mm").trim();
            let formatted_fl = if let Ok(val) = clean_fl.parse::<f64>() {
                format!("{:.2} mm", val)
            } else {
                format!("{} mm", clean_fl)
            };
            out.push_str(&format!(
                "    {:<wid2$} : {}\n",
                "Focal Length", formatted_fl
            ));
            found_any = true;
        }
        let has_gps = if exif_data.gps_present { "Yes" } else { "No" }.to_string();
        out.push_str(&format!("    {:<wid2$} : {}\n", "GPS Data".red(), has_gps));

        if !found_any {
            out.push_str(&format!(
                "    {}\n",
                "No standard camera tags found in EXIF"
            ));
        }
    } else {
        out.push_str(&format!("    {}\n", "None (or unreadable EXIF header)"));
    }
    out.push('\n'); // Blank line after each file
    out
}

/// Build a detailed info string for a single image file.
pub fn format_info(file: &Path) -> String {
    let mut out = String::new();

    // File system metadata
    out.push_str(&format_file_metadata(file));

    // Format detection
    let is_heic_file = heic::is_heic(file);
    out.push_str(&format_format(file, is_heic_file));

    // Decode image to get dimensions and technical details
    let raw_bytes = std::fs::read(file).unwrap_or_default();
    let (img, img_details) = if is_heic_file {
        if let Ok((decoded_img, _, _)) = heic::decode(file) {
            let details = exif::get_image_details(decoded_img.color(), "HEIC", &raw_bytes);
            (Some(decoded_img), details)
        } else {
            (
                None,
                exif::get_image_details(ColorType::Rgb8, "HEIC", &raw_bytes),
            )
        }
    } else {
        if let Ok(img) = image::open(file) {
            let format_str = image::ImageFormat::from_path(file)
                .map(|f| format!("{:?}", f).to_uppercase())
                .unwrap_or_else(|_| "UNKNOWN".to_string());
            let details = exif::get_image_details(img.color(), &format_str, &raw_bytes);
            (Some(img), details)
        } else {
            (
                None,
                exif::get_image_details(ColorType::Rgb8, "UNKNOWN", &raw_bytes),
            )
        }
    };

    // Dimensions
    if let Some(ref img) = img {
        out.push_str(&format_dimensions(img));
    }

    // Technical details
    out.push_str(&format_image_details(&img_details));

    // EXIF metadata
    out.push_str(&format_exif_metadata(file));

    out
}
