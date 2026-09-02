mod common;

#[cfg(test)]
mod tests {
    use super::common::{create_test_heic, create_test_jpeg, create_test_png};
    use bat_img_rs::info::{
        easy_file_sz, format_dimensions, format_exif_metadata, format_file_metadata, format_format,
        format_image_details, format_info,
    };
    use std::path::Path;
    use tempfile::tempdir;

    // ---------- easy_file_sz ----------
    #[test]
    fn test_bytes() {
        assert_eq!(easy_file_sz(0), "0 B");
        assert_eq!(easy_file_sz(512), "512 B");
        assert_eq!(easy_file_sz(1023), "1023 B");
    }

    #[test]
    fn test_kilobytes() {
        assert_eq!(easy_file_sz(1024), "1 KB");
        assert_eq!(easy_file_sz(1500), "1 KB");
        assert_eq!(easy_file_sz(2000), "2 KB");
    }

    #[test]
    fn test_megabytes() {
        assert_eq!(easy_file_sz(1_048_576), "1.0 MB");
        assert_eq!(easy_file_sz(1_572_864), "1.5 MB");
    }

    #[test]
    fn test_gigabytes() {
        assert_eq!(easy_file_sz(1_073_741_824), "1.0 GB");
        assert_eq!(easy_file_sz(2_684_354_560), "2.5 GB");
    }

    #[test]
    fn test_terabyte_overflow() {
        assert_eq!(easy_file_sz(1_099_511_627_776), "1024.0 GB");
    }

    // ---------- format_file_metadata ----------
    #[test]
    fn file_metadata_contains_size_and_date() {
        let dir = tempdir().unwrap();
        let path = create_test_jpeg(&dir, "test.jpg");
        let metadata = format_file_metadata(&path);
        assert!(metadata.contains("File Size"));
        assert!(metadata.contains("Last Modified"));
        assert!(metadata.contains("bytes"));
    }

    #[test]
    fn file_metadata_missing_file_is_empty() {
        let metadata = format_file_metadata(Path::new("/nonexistent/file.jpg"));
        assert!(metadata.is_empty());
    }

    // ---------- format_format ----------
    #[test]
    fn format_detects_jpeg() {
        let dir = tempdir().unwrap();
        let path = create_test_jpeg(&dir, "test.jpg");
        let format = format_format(&path, false);
        assert!(format.to_lowercase().contains("jpeg"));
    }

    #[test]
    fn format_detects_png() {
        let dir = tempdir().unwrap();
        let path = create_test_png(&dir, "test.png");
        let format = format_format(&path, false);
        assert!(format.to_lowercase().contains("png"));
    }

    #[test]
    fn format_detects_heic() {
        let dir = tempdir().unwrap();
        let Some(path) = create_test_heic(&dir, "test.heic") else {
            println!("Skipping: libheif encoding failed.");
            return;
        };
        let format = format_format(&path, true);
        assert!(format.contains("HEIF"));
    }

    // ---------- format_dimensions ----------
    #[test]
    fn dimensions_show_megapixels() {
        let dir = tempdir().unwrap();
        let path = create_test_png(&dir, "test.png");
        let img = image::open(&path).unwrap();
        let dims = format_dimensions(&img);
        assert!(dims.contains("100 x 100"));
        assert!(dims.contains("0.0 MP"));
    }

    // ---------- format_image_details ----------
    #[test]
    fn image_details_include_key_fields() {
        let dir = tempdir().unwrap();
        let path = create_test_png(&dir, "test.png");
        let img = image::open(&path).unwrap();
        let raw_bytes = std::fs::read(&path).unwrap();
        let details = bat_img_rs::exif::get_image_details(img.color(), "PNG", &raw_bytes);
        let out = format_image_details(&details);
        assert!(out.contains("Bit Depth"));
        assert!(out.contains("Alpha Channel"));
        assert!(out.contains("Color Space"));
        assert!(out.contains("Color Profile"));
    }

    // ---------- format_exif_metadata ----------
    #[test]
    fn exif_metadata_has_section_header() {
        let dir = tempdir().unwrap();
        let path = create_test_jpeg(&dir, "test.jpg");
        let exif = format_exif_metadata(&path);
        assert!(exif.contains("[ EXIF Metadata ]"));

        let has_any_tag = [
            "Make",
            "Model",
            "Date/Time",
            "ISO Speed",
            "Exposure",
            "Aperture",
            "Focal Length",
            "GPS Data",
        ]
        .iter()
        .any(|tag| exif.contains(tag));
        let has_fallback = exif.contains("No standard camera tags")
            || exif.contains("None (or unreadable EXIF header)");
        assert!(has_any_tag || has_fallback);
    }

    // ---------- format_info (integration) ----------
    #[test]
    fn jpeg_info_contains_key_fields() {
        let dir = tempdir().unwrap();
        let path = create_test_jpeg(&dir, "test.jpg");
        let info = format_info(&path);

        assert!(info.contains("File Size"));
        assert!(info.contains("Last Modified"));
        assert!(info.contains("Format"));
        assert!(info.to_lowercase().contains("jpeg"));
        assert!(info.contains("Dimensions"));
        assert!(info.contains("100 x 100"));
        assert!(info.contains("Bit Depth"));
        assert!(info.contains("Alpha Channel"));
        assert!(info.contains("Color Space"));
        assert!(info.contains("Color Profile"));
        assert!(info.contains("Chroma Format"));
    }

    #[test]
    fn png_info_contains_key_fields() {
        let dir = tempdir().unwrap();
        let path = create_test_png(&dir, "test.png");
        let info = format_info(&path);

        assert!(info.contains("File Size"));
        assert!(info.contains("Last Modified"));
        assert!(info.contains("Format"));
        assert!(info.to_lowercase().contains("png"));
        assert!(info.contains("Dimensions"));
        assert!(info.contains("100 x 100"));
        assert!(info.contains("Bit Depth"));
        assert!(info.contains("Alpha Channel"));
        assert!(info.contains("Color Space"));
        assert!(info.contains("Color Profile"));
        assert!(info.contains("Chroma Format"));
    }

    #[test]
    fn heic_info_works_if_encoder_available() {
        let dir = tempdir().unwrap();
        let Some(path) = create_test_heic(&dir, "test.heic") else {
            println!("Skipping test: libheif encoding failed.");
            return;
        };
        let info = format_info(&path);

        assert!(info.contains("File Size"));
        assert!(info.contains("Last Modified"));
        assert!(info.contains("Format"));
        assert!(info.contains("HEIF"));
        assert!(info.contains("Dimensions"));
        assert!(info.contains("100 x 100"));
        assert!(info.contains("Bit Depth"));
        assert!(info.contains("Alpha Channel"));
        assert!(info.contains("Color Space"));
        assert!(info.contains("Color Profile"));
        assert!(info.contains("Chroma Format"));
    }

    #[test]
    fn missing_file_does_not_panic() {
        let info = format_info(Path::new("/nonexistent/file.jpg"));
        assert!(!info.is_empty());
        assert!(!info.contains("panic"));
    }
}
