mod common;

#[cfg(test)]
mod tests {
    use super::common::{create_test_heic, create_test_jpeg, create_test_png};
    use bat_img_rs::info::format_info;
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn jpeg_info_contains_key_fields() {
        let dir = tempdir().unwrap();
        let path = create_test_jpeg(&dir, "test.jpg");
        let info = format_info(&path);

        assert!(info.contains("File Size"));
        assert!(info.contains("Last Modified"));
        assert!(info.contains("Format"));
        // Case‑insensitive check for "jpeg"
        assert!(info.to_lowercase().contains("jpeg"));
        assert!(info.contains("Dimensions"));
        assert!(info.contains("100 x 100"));
        assert!(info.contains("Bit Depth"));
        assert!(info.contains("Alpha Channel"));
        assert!(info.contains("Colorspace"));
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
        // Case‑insensitive check for "png"
        assert!(info.to_lowercase().contains("png"));
        assert!(info.contains("Dimensions"));
        assert!(info.contains("100 x 100"));
        assert!(info.contains("Bit Depth"));
        assert!(info.contains("Alpha Channel"));
        assert!(info.contains("Colorspace"));
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
        assert!(info.contains("Colorspace"));
        assert!(info.contains("Chroma Format"));
    }

    #[test]
    fn missing_file_does_not_panic() {
        let info = bat_img_rs::info::format_info(Path::new("/nonexistent/file.jpg"));
        assert!(!info.is_empty());
        assert!(!info.contains("panic"));
    }
}
