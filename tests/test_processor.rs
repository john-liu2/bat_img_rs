/// Integration tests for bat_img_rs image-processing operations.
///
/// These tests exercise the full per-file pipeline end-to-end by creating
/// real temporary images on disk and running ProcessingContext::process().
#[cfg(test)]
mod tests {
    use bat_img_rs::pipeline::{Pipeline, ResizeSpec};
    use bat_img_rs::processor::ProcessingContext;
    use image::{DynamicImage, GenericImageView, RgbImage, Rgba};
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Solid-colour RGB test image.
    fn solid_rgb(w: u32, h: u32, r: u8, g: u8, b: u8) -> DynamicImage {
        let mut img = RgbImage::new(w, h);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgb([r, g, b]);
        }
        DynamicImage::ImageRgb8(img)
    }

    /// Save an image as JPEG and return its path.
    fn save_jpeg(img: &DynamicImage, dir: &TempDir, name: &str) -> PathBuf {
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
    fn save_png(img: &DynamicImage, dir: &TempDir, name: &str) -> PathBuf {
        let path = dir.path().join(name);
        img.save(&path).unwrap();
        path
    }

    /// Build a minimal Pipeline with everything disabled, pointing output at `out_dir`.
    fn base_pipeline(out_dir: PathBuf) -> Pipeline {
        Pipeline {
            strip_gps: false,
            strip_all: false,
            auto_orient: false,
            resize: None,
            no_upscale: false,
            filter: image::imageops::FilterType::Lanczos3,
            rotate: None,
            flip_h: false,
            flip_v: false,
            border_px: None,
            border_rgba: None,
            brightness: None,
            contrast: None,
            sharpen: false,
            grayscale: false,
            output_format: None,
            quality: None,
            output_dir: Some(out_dir),
            in_place: false,
            prefix: String::new(),
            suffix: String::new(),
            overwrite: true,
            dry_run: false,
        }
    }

    /// Run a pipeline on `input_path`, return the output path.
    fn run(input_path: PathBuf, pipeline: Pipeline) -> PathBuf {
        let ctx = ProcessingContext {
            input_path,
            pipeline: Arc::new(pipeline),
        };
        ctx.process().expect("processing failed")
    }

    // ── Resize ────────────────────────────────────────────────────────────────

    #[test]
    fn resize_width_fixed_height_auto() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let src = save_jpeg(&solid_rgb(1000, 500, 255, 0, 0), &tmp, "src.jpg");

        let mut p = base_pipeline(out.path().to_path_buf());
        p.resize = Some(ResizeSpec {
            width: 200,
            height: 0,
        });

        let output = run(src, p);
        let img = image::open(&output).unwrap();
        assert_eq!(img.width(), 200);
        assert_eq!(img.height(), 100); // aspect preserved
    }

    #[test]
    fn resize_height_fixed_width_auto() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let src = save_jpeg(&solid_rgb(800, 400, 0, 255, 0), &tmp, "src.jpg");

        let mut p = base_pipeline(out.path().to_path_buf());
        p.resize = Some(ResizeSpec {
            width: 0,
            height: 100,
        });

        let output = run(src, p);
        let img = image::open(&output).unwrap();
        assert_eq!(img.height(), 100);
        assert_eq!(img.width(), 200); // aspect preserved
    }

    #[test]
    fn resize_both_dimensions_exact() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let src = save_jpeg(&solid_rgb(640, 480, 0, 0, 255), &tmp, "src.jpg");

        let mut p = base_pipeline(out.path().to_path_buf());
        p.resize = Some(ResizeSpec {
            width: 100,
            height: 50,
        });

        let output = run(src, p);
        let img = image::open(&output).unwrap();
        assert_eq!(img.width(), 100);
        assert_eq!(img.height(), 50);
    }

    #[test]
    fn no_upscale_skips_when_image_is_smaller() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let src = save_jpeg(&solid_rgb(100, 100, 128, 128, 128), &tmp, "src.jpg");

        let mut p = base_pipeline(out.path().to_path_buf());
        p.resize = Some(ResizeSpec {
            width: 5000,
            height: 0,
        });
        p.no_upscale = true;

        let output = run(src, p);
        let img = image::open(&output).unwrap();
        // Should not have been upscaled
        assert_eq!(img.width(), 100);
        assert_eq!(img.height(), 100);
    }

    // ── Rotation ──────────────────────────────────────────────────────────────

    #[test]
    fn rotate_90_swaps_dimensions() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        // Non-square so rotation is detectable via dimensions
        let src = save_jpeg(&solid_rgb(200, 100, 0, 0, 0), &tmp, "src.jpg");

        let mut p = base_pipeline(out.path().to_path_buf());
        p.rotate = Some(90);

        let output = run(src, p);
        let img = image::open(&output).unwrap();
        assert_eq!(img.width(), 100);
        assert_eq!(img.height(), 200);
    }

    #[test]
    fn rotate_180_preserves_dimensions() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let src = save_jpeg(&solid_rgb(200, 100, 0, 0, 0), &tmp, "src.jpg");

        let mut p = base_pipeline(out.path().to_path_buf());
        p.rotate = Some(180);

        let output = run(src, p);
        let img = image::open(&output).unwrap();
        assert_eq!(img.width(), 200);
        assert_eq!(img.height(), 100);
    }

    #[test]
    fn rotate_270_swaps_dimensions() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let src = save_jpeg(&solid_rgb(200, 100, 0, 0, 0), &tmp, "src.jpg");

        let mut p = base_pipeline(out.path().to_path_buf());
        p.rotate = Some(270);

        let output = run(src, p);
        let img = image::open(&output).unwrap();
        assert_eq!(img.width(), 100);
        assert_eq!(img.height(), 200);
    }

    // ── Flip ──────────────────────────────────────────────────────────────────

    #[test]
    fn flip_horizontal_mirrors_pixels() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();

        // Left half red, right half blue — flipping should swap them
        let mut img = RgbImage::new(4, 2);
        for y in 0..2 {
            for x in 0..2 {
                img.put_pixel(x, y, image::Rgb([255, 0, 0])); // left = red
                img.put_pixel(x + 2, y, image::Rgb([0, 0, 255])); // right = blue
            }
        }
        let src_img = DynamicImage::ImageRgb8(img);
        let src = save_jpeg(&src_img, &tmp, "src.jpg");

        let mut p = base_pipeline(out.path().to_path_buf());
        p.flip_h = true;

        let output = run(src, p);
        let result = image::open(&output).unwrap();
        let pixel_left = result.get_pixel(0, 0);
        let pixel_right = result.get_pixel(3, 0);

        // After flip: left should be blue-ish, right should be red-ish
        // (JPEG lossy so check dominant channel)
        assert!(
            pixel_left[2] > pixel_left[0],
            "left should be blue after hflip"
        );
        assert!(
            pixel_right[0] > pixel_right[2],
            "right should be red after hflip"
        );
    }

    #[test]
    fn flip_vertical_preserves_dimensions() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let src = save_jpeg(&solid_rgb(80, 60, 100, 100, 100), &tmp, "src.jpg");

        let mut p = base_pipeline(out.path().to_path_buf());
        p.flip_v = true;

        let output = run(src, p);
        let img = image::open(&output).unwrap();
        assert_eq!(img.dimensions(), (80, 60));
    }

    // ── Border ────────────────────────────────────────────────────────────────

    #[test]
    fn border_increases_dimensions() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let src = save_png(&solid_rgb(100, 80, 200, 200, 200), &tmp, "src.png");

        let mut p = base_pipeline(out.path().to_path_buf());
        p.border_px = Some(10);
        p.border_rgba = Some(Rgba([0, 0, 0, 255]));

        let output = run(src, p);
        let img = image::open(&output).unwrap();
        assert_eq!(img.width(), 120); // 100 + 10*2
        assert_eq!(img.height(), 100); // 80  + 10*2
    }

    #[test]
    fn border_corner_pixel_matches_border_color() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let src = save_png(&solid_rgb(50, 50, 128, 128, 128), &tmp, "src.png");

        let mut p = base_pipeline(out.path().to_path_buf());
        p.border_px = Some(5);
        p.border_rgba = Some(Rgba([255, 0, 0, 255])); // red border

        let output = run(src, p);
        let img = image::open(&output).unwrap();
        let corner = img.get_pixel(0, 0);
        // Top-left corner is in the border — should be red
        assert!(corner[0] > 200, "corner R should be high (red border)");
        assert!(corner[1] < 50, "corner G should be low (red border)");
        assert!(corner[2] < 50, "corner B should be low (red border)");
    }

    // ── Grayscale ─────────────────────────────────────────────────────────────

    #[test]
    fn grayscale_output_has_equal_rgb_channels() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        // Vivid green source — after grayscale R=G=B
        let src = save_png(&solid_rgb(20, 20, 0, 200, 0), &tmp, "src.png");

        let mut p = base_pipeline(out.path().to_path_buf());
        p.grayscale = true;

        let output = run(src, p);
        let img = image::open(&output).unwrap().to_rgb8();
        let p0 = img.get_pixel(10, 10);
        // All three channels equal after grayscale (luma conversion)
        assert_eq!(p0[0], p0[1]);
        assert_eq!(p0[1], p0[2]);
    }

    // ── Format conversion ─────────────────────────────────────────────────────

    #[test]
    fn jpeg_input_produces_jpeg_output_by_default() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let src = save_jpeg(&solid_rgb(64, 64, 0, 0, 0), &tmp, "photo.jpg");

        let output = run(src, base_pipeline(out.path().to_path_buf()));
        assert_eq!(output.extension().unwrap(), "jpg");
        // Must be decodable
        image::open(&output).unwrap();
    }

    #[test]
    fn png_input_produces_png_output_by_default() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let src = save_png(&solid_rgb(64, 64, 0, 0, 0), &tmp, "image.png");

        let output = run(src, base_pipeline(out.path().to_path_buf()));
        assert_eq!(output.extension().unwrap(), "png");
        image::open(&output).unwrap();
    }

    #[test]
    fn format_conversion_jpeg_to_png() {
        use bat_img_rs::cli::OutputFormat;
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let src = save_jpeg(&solid_rgb(32, 32, 0, 0, 0), &tmp, "photo.jpg");

        let mut p = base_pipeline(out.path().to_path_buf());
        p.output_format = Some(OutputFormat::Png);

        let output = run(src, p);
        assert_eq!(output.extension().unwrap(), "png");
        image::open(&output).unwrap();
    }

    // ── Prefix / Suffix ───────────────────────────────────────────────────────

    #[test]
    fn prefix_applied_to_output_filename() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let src = save_jpeg(&solid_rgb(10, 10, 0, 0, 0), &tmp, "photo.jpg");

        let mut p = base_pipeline(out.path().to_path_buf());
        p.prefix = "web_".to_string();

        let output = run(src, p);
        assert!(
            output
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("web_")
        );
    }

    #[test]
    fn suffix_applied_to_output_filename() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let src = save_jpeg(&solid_rgb(10, 10, 0, 0, 0), &tmp, "photo.jpg");

        let mut p = base_pipeline(out.path().to_path_buf());
        p.suffix = "_sm".to_string();

        let output = run(src, p);
        let stem = output.file_stem().unwrap().to_string_lossy();
        assert!(stem.ends_with("_sm"), "stem was: {stem}");
    }

    // ── In-place mode ─────────────────────────────────────────────────────────

    #[test]
    fn in_place_mode_overwrites_original() {
        let tmp = TempDir::new().unwrap();
        let src = save_jpeg(&solid_rgb(200, 100, 128, 0, 0), &tmp, "photo.jpg");
        let original_size = std::fs::metadata(&src).unwrap().len();

        let mut p = base_pipeline(PathBuf::new()); // output_dir unused in-place
        p.in_place = true;
        p.output_dir = None;
        p.resize = Some(ResizeSpec {
            width: 50,
            height: 0,
        });

        let output = run(src.clone(), p);
        // Output path must equal input path
        assert_eq!(output, src);
        // File must exist and have changed (smaller after resize)
        let new_size = std::fs::metadata(&output).unwrap().len();
        // Resized image should produce a different (typically smaller) file
        assert_ne!(new_size, original_size);
        // Must still be a valid image at the new dimensions
        let img = image::open(&output).unwrap();
        assert_eq!(img.width(), 50);
    }

    #[test]
    fn in_place_strips_gps_and_file_remains_valid() {
        let tmp = TempDir::new().unwrap();
        let src = save_jpeg(&solid_rgb(64, 64, 0, 128, 0), &tmp, "photo.jpg");

        let mut p = base_pipeline(PathBuf::new());
        p.in_place = true;
        p.output_dir = None;
        p.strip_gps = true;

        let output = run(src.clone(), p);
        assert_eq!(output, src);
        image::open(&output).unwrap(); // must still be decodable
    }

    // ── Dry-run ───────────────────────────────────────────────────────────────

    #[test]
    fn dry_run_does_not_create_output_file() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let src = save_jpeg(&solid_rgb(50, 50, 0, 0, 128), &tmp, "photo.jpg");

        let mut p = base_pipeline(out.path().to_path_buf());
        p.dry_run = true;

        let output = run(src, p);
        assert!(!output.exists(), "dry-run must not write any file");
    }

    // ── Overwrite guard ───────────────────────────────────────────────────────

    #[test]
    fn overwrite_false_skips_existing_output() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let src = save_jpeg(&solid_rgb(50, 50, 0, 0, 0), &tmp, "photo.jpg");

        // First run — creates output
        let mut p = base_pipeline(out.path().to_path_buf());
        p.overwrite = true;
        let output = run(src.clone(), p);
        let size_after_first = std::fs::metadata(&output).unwrap().len();

        // Second run with overwrite=false and a different transform — should be skipped
        let mut p2 = base_pipeline(out.path().to_path_buf());
        p2.overwrite = false;
        p2.resize = Some(ResizeSpec {
            width: 10,
            height: 0,
        });
        run(src, p2);

        // File size should be unchanged (second run skipped)
        let size_after_second = std::fs::metadata(&output).unwrap().len();
        assert_eq!(size_after_first, size_after_second);
    }

    #[test]
    fn overwrite_true_replaces_existing_output() {
        let tmp = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let src = save_jpeg(&solid_rgb(200, 200, 100, 100, 100), &tmp, "photo.jpg");

        // First run — full-size output
        let mut p1 = base_pipeline(out.path().to_path_buf());
        p1.overwrite = true;
        let output = run(src.clone(), p1);
        let img1 = image::open(&output).unwrap();
        let dims1 = img1.dimensions();

        // Second run — resize to 10x10 with overwrite=true
        let mut p2 = base_pipeline(out.path().to_path_buf());
        p2.overwrite = true;
        p2.resize = Some(ResizeSpec {
            width: 10,
            height: 0,
        });
        run(src, p2);

        let img2 = image::open(&output).unwrap();
        assert_ne!(
            img2.dimensions(),
            dims1,
            "overwrite=true should replace the file"
        );
        assert_eq!(img2.width(), 10);
    }
}
