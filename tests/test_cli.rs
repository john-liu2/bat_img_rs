mod common;

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::path::PathBuf;
    use tempfile::{tempdir, TempDir};
    use image::RgbImage;

    fn create_test_jpeg(dir: &TempDir, name: &str) -> PathBuf {
        let path = dir.path().join(name);
        let img = RgbImage::from_pixel(100, 100, image::Rgb([255, 0, 0]));
        img.save(&path).unwrap();
        path
    }

    fn create_test_png(dir: &TempDir, name: &str) -> PathBuf {
        let path = dir.path().join(name);
        let img = RgbImage::from_pixel(100, 100, image::Rgb([0, 255, 0]));
        img.save(&path).unwrap();
        path
    }

    fn run_info(path: &PathBuf) -> String {
        let output = Command::new("cargo")
            .arg("run")
            .arg("--")
            .arg("--info")
            .arg("--input")
            .arg(path)
            .output()
            .expect("failed to run command");

        if !output.status.success() {
            eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
            eprintln!("stdout: {}", String::from_utf8_lossy(&output.stdout));
            panic!("command failed with status: {}", output.status);
        }
        let stdout = String::from_utf8(output.stdout).unwrap();
        // Print for debugging if needed (optional)
        // eprintln!("stdout:\n{}", stdout);
        stdout
    }

    #[test]
    fn info_shows_details_for_jpeg() {
        let dir = tempdir().unwrap();
        let path = create_test_jpeg(&dir, "test.jpg");
        let stdout = run_info(&path);

        assert!(stdout.contains("Dimensions") && stdout.contains("100x100"));
        assert!(stdout.contains("Bit Depth") && stdout.contains("8 bits/channel"));
        assert!(stdout.contains("Alpha Channel") && stdout.contains("No"));
        assert!(stdout.contains("Colorspace") && stdout.contains("YCbCr"));
        assert!(stdout.contains("Chroma Format")); // value may vary (e.g., 4:2:0, 4:4:4)
    }

    #[test]
    fn info_shows_details_for_png() {
        let dir = tempdir().unwrap();
        let path = create_test_png(&dir, "test.png");
        let stdout = run_info(&path);

        assert!(stdout.contains("Dimensions") && stdout.contains("100x100"));
        assert!(stdout.contains("Bit Depth") && stdout.contains("8 bits/channel"));
        assert!(stdout.contains("Alpha Channel") && stdout.contains("No"));
        assert!(stdout.contains("Colorspace") && stdout.contains("RGB")); // PNG decoded as RGB
        assert!(stdout.contains("Chroma Format") && stdout.contains("4:4:4"));
    }

    #[test]
    fn info_shows_details_for_heic() {
        use bat_img_rs::heic;
        use libheif_rs::CompressionFormat;

        let dir = tempdir().unwrap();
        let path = dir.path().join("test.heic");
        let img = image::DynamicImage::ImageRgb8(RgbImage::from_pixel(100, 100, image::Rgb([0, 0, 255])));
        if heic::encode(&img, &path, CompressionFormat::Hevc, Some(80), None).is_err() {
            // Skip if encoding fails (libheif not installed)
            return;
        }

        let stdout = run_info(&path);

        assert!(stdout.contains("Dimensions") && stdout.contains("100x100"));
        assert!(stdout.contains("Bit Depth") && stdout.contains("8 bits/channel"));
        assert!(stdout.contains("Alpha Channel") && stdout.contains("No"));
        assert!(stdout.contains("Colorspace") && stdout.contains("YCbCr"));
        assert!(stdout.contains("Chroma Format")); // value depends on encoder
    }
}
