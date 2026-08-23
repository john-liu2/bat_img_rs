// info.rs - read image files info
use crate::exif;
use crate::heic;

use colored::Colorize;
use image::ColorType;
use std::path::Path;

const TAB_WIDTH: usize = 15;

/// Build a detailed info string for a single image file.
pub fn format_info(file: &Path) -> String {
    let width = TAB_WIDTH;
    let mut out = String::new();

    // File system metadata
    if let Ok(metadata) = std::fs::metadata(file) {
        let bytes = metadata.len();
        let readable_size = if bytes >= 1_048_576 {
            format!("{:.1} MB", bytes as f64 / 1_048_576.0)
        } else if bytes >= 1024 {
            format!("{:.0} KB", bytes as f64 / 1024.0)
        } else {
            format!("{} B", bytes)
        };
        out.push_str(&format!(
            "  {:<width$} : {} ({})\n",
            "File Size".bold(),
            readable_size,
            format!("{} bytes", bytes).dimmed()
        ));
    }

    // Format
    let is_heic_file = heic::is_heic(file);
    let format_str = if is_heic_file {
        "HEIC".to_string()
    } else {
        image::ImageFormat::from_path(file)
            .map(|f| format!("{:?}", f))
            .unwrap_or_else(|_| "Unknown".to_string())
    };
    out.push_str(&format!(
        "  {:<width$} : {}\n",
        "Format".bold(),
        format_str.blue()
    ));

    // Decode to get pixel details
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
            let details =
                exif::get_image_details(img.color(), &format_str.to_uppercase(), &raw_bytes);
            (Some(img), details)
        } else {
            (
                None,
                exif::get_image_details(
                    ColorType::Rgb8,
                    &format_str.to_uppercase(),
                    &raw_bytes,
                ),
            )
        }
    };

    if let Some(ref img) = img {
        let (w, h) = (img.width(), img.height());
        let megapixels = (w as f64 * h as f64) / 1_000_000.0;
        out.push_str(&format!(
            "  {:<width$} : {}x{} ({:.1} MP)\n",
            "Dimensions".bold(),
            w,
            h,
            megapixels
        ));
    }

    out.push_str(&format!(
        "  {:<width$} : {}\n",
        "Bit Depth".bold(),
        img_details.bit_depth
    ));
    out.push_str(&format!(
        "  {:<width$} : {}\n",
        "Alpha Channel".bold(),
        if img_details.has_alpha { "Yes" } else { "No" }
    ));
    out.push_str(&format!(
        "  {:<width$} : {}\n",
        "Colorspace".bold(),
        img_details.colorspace
    ));
    if let Some(chroma) = img_details.chroma_format {
        out.push_str(&format!(
            "  {:<width$} : {}\n",
            "Chroma Format".bold(),
            chroma
        ));
    }

    // EXIF metadata
    out.push_str("\n  ");
    out.push_str(&"[ EXIF Metadata ]".bold().underline().dimmed().to_string());
    out.push('\n');
    if let Some(exif_data) = exif::read_exif(file) {
        let mut found_any = false;

        if let Some(ref make) = exif_data.make {
            out.push_str(&format!("    {:<width$} : {}\n", "Make", make));
            found_any = true;
        }
        if let Some(ref model) = exif_data.model {
            out.push_str(&format!("    {:<width$} : {}\n", "Model", model));
            found_any = true;
        }
        if let Some(ref date) = exif_data.date_time {
            out.push_str(&format!("    {:<width$} : {}\n", "Date/Time", date));
            found_any = true;
        }
        if let Some(ref iso) = exif_data.iso {
            out.push_str(&format!("    {:<width$} : ISO {}\n", "ISO Speed", iso));
            found_any = true;
        }
        if let Some(ref exp) = exif_data.exposure {
            let clean_exp = exp.trim().trim_end_matches('s').trim();
            out.push_str(&format!("    {:<width$} : {} s\n", "Exposure", clean_exp));
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
            out.push_str(&format!("    {:<width$} : f/{}\n", "Aperture", formatted_f));
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
                "    {:<width$} : {}\n",
                "Focal Length",
                formatted_fl
            ));
            found_any = true;
        }
        let has_gps = if exif_data.gps_present {"Present"} else {"None"}.to_string();
        out.push_str(&format!("    {:<width$} : {}\n", "GPS Data".red(), has_gps));

        if !found_any {
            out.push_str(&format!("    {}\n", "No standard camera tags found in EXIF."));
        }
    } else {
        out.push_str(&format!("    {}\n", "None (or unreadable EXIF header)"));
    }
    out.push('\n'); // Blank line after each file
    out
}
