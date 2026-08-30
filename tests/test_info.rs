mod common;

#[cfg(test)]
mod tests {
    use super::common::{create_test_heic, create_test_jpeg, create_test_png};
    use bat_img_rs::info::{easy_file_sz, format_info};
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn test_bytes() {
        assert_eq!(easy_file_sz(0), "0 B");
        assert_eq!(easy_file_sz(512), "512 B");
        assert_eq!(easy_file_sz(1023), "1023 B");
    }

    #[test]
    fn test_kilobytes() {
        assert_eq!(easy_file_sz(1024), "1 KB");
        assert_eq!(easy_file_sz(1500), "1 KB"); // 1500 / 1024 = 1.46... rounds to 1
        assert_eq!(easy_file_sz(2000), "2 KB"); // 2000 / 1024 = 1.95... rounds to 2
    }

    #[test]
    fn test_megabytes() {
        // Exact MB bound
        assert_eq!(easy_file_sz(1_048_576), "1.0 MB");
        // 1.5 MB
        assert_eq!(easy_file_sz(1_572_864), "1.5 MB");
    }

    #[test]
    fn test_gigabytes() {
        // Exact GB bound
        assert_eq!(easy_file_sz(1_073_741_824), "1.0 GB");
        // 2.5 GB
        assert_eq!(easy_file_sz(2_684_354_560), "2.5 GB");
    }

    #[test]
    fn test_terabyte_overflow() {
        // 1 TB (1024 * 1024 * 1024 * 1024)
        // Checks that the loop stops at GB and doesn't divide an extra time
        assert_eq!(easy_file_sz(1_099_511_627_776), "1024.0 GB");
    }

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
