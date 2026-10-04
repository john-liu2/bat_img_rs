/// Test bat_img_rs::exif::image_details
mod common;

#[cfg(test)]
mod tests {
    use super::common::{build_tiff_le, create_test_heic, jpeg_with_exif};
    use bat_img_rs::exif::get_image_details_with_grayscale;
    use image::{ColorType, RgbImage};
    use tempfile::tempdir;

    const GRAY_PROFILE: &str = "Generic Gray Gamma 2.2 Profile";

    #[test]
    fn grayscale_heic_reports_gray_when_decoded_as_rgb() {
        let details = get_image_details_with_grayscale(ColorType::Rgb8, "HEIC", &[], true);

        assert_eq!(details.colorspace, "Gray");
        assert_eq!(details.c_profile, "Generic Gray Gamma 2.2 Profile");
    }
    #[test]
    fn get_image_details_grayscale_heic_uses_gray_profile() {
        let details = get_image_details_with_grayscale(ColorType::L8, "HEIC", &[], false);

        assert_eq!(details.colorspace, "Gray");
        assert_eq!(details.c_profile, GRAY_PROFILE);
        assert!(!details.has_alpha);
        assert_eq!(details.bit_depth, "8 bits/channel");
    }

    #[test]
    fn get_image_details_grayscale_jpeg_uses_gray_profile() {
        let details = get_image_details_with_grayscale(ColorType::L8, "JPEG", &[], false);

        assert_eq!(details.colorspace, "Gray");
        assert_eq!(details.c_profile, GRAY_PROFILE);
        assert!(!details.has_alpha);
        assert_eq!(details.bit_depth, "8 bits/channel");
    }

    #[test]
    fn get_image_details_grayscale_tiff_uses_gray_profile() {
        let details = get_image_details_with_grayscale(ColorType::L8, "TIFF", &[], false);

        assert_eq!(details.colorspace, "Gray");
        assert_eq!(details.c_profile, GRAY_PROFILE);
        assert!(!details.has_alpha);
        assert_eq!(details.bit_depth, "8 bits/channel");
    }

    #[test]
    fn get_image_details_grayscale_png_uses_gray_profile() {
        let details = get_image_details_with_grayscale(ColorType::L8, "PNG", &[], false);

        assert_eq!(details.colorspace, "Gray");
        assert_eq!(details.c_profile, GRAY_PROFILE);
        assert!(!details.has_alpha);
        assert_eq!(details.bit_depth, "8 bits/channel");
    }

    #[test]
    fn get_image_details_grayscale_16bit_uses_gray_profile() {
        for format in ["HEIC", "JPEG", "TIFF", "PNG"] {
            let details = get_image_details_with_grayscale(ColorType::L16, format, &[], false);

            assert_eq!(details.colorspace, "Gray", "wrong colorspace for {format}");
            assert_eq!(
                details.c_profile, GRAY_PROFILE,
                "wrong color profile for {format}"
            );
            assert_eq!(details.bit_depth, "16 bits/channel");
        }
    }

    #[test]
    fn get_image_details_rgb_still_uses_color_profile_logic() {
        let details = get_image_details_with_grayscale(ColorType::Rgb8, "PNG", &[], false);

        assert_eq!(details.colorspace, "RGB");

        // With no embedded profile, the existing fallback remains sRGB.
        assert_eq!(details.c_profile, "sRGB");
    }

    #[test]
    fn get_image_details_rgba_still_reports_rgba() {
        let details = get_image_details_with_grayscale(ColorType::Rgba8, "PNG", &[], false);

        assert_eq!(details.colorspace, "RGBA");
        assert_eq!(details.c_profile, "sRGB");
        assert!(details.has_alpha);
    }

    #[test]
    fn image_details_jpeg_exif_color_space_no_icc_srgb() {
        let tiff = build_tiff_le(&[(0xA001, 3, 1)]);
        let jpeg = jpeg_with_exif(&tiff);
        let details = get_image_details_with_grayscale(ColorType::Rgb8, "JPEG", &jpeg, false);
        assert_eq!(details.c_profile, "sRGB IEC61966-2.1");
    }

    #[test]
    fn image_details_jpeg_exif_color_space_no_icc_adobe_rgb() {
        let tiff = build_tiff_le(&[(0xA001, 3, 2)]);
        let jpeg = jpeg_with_exif(&tiff);
        let details = get_image_details_with_grayscale(ColorType::Rgb8, "JPEG", &jpeg, false);
        assert_eq!(details.c_profile, "Adobe RGB (1998)");
    }

    #[test]
    fn image_details_jpeg_exif_color_space_unknown_default() {
        let tiff = build_tiff_le(&[(0xA001, 3, 0)]);
        let jpeg = jpeg_with_exif(&tiff);
        let details = get_image_details_with_grayscale(ColorType::Rgb8, "JPEG", &jpeg, false);
        assert_eq!(details.c_profile, "sRGB");
    }

    #[test]
    fn image_details_png_with_exif_color_space_does_not_override_icc() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let chunk_data = b"sRGB\0\0fake_compressed_payload";
        png.extend_from_slice(&(chunk_data.len() as u32).to_be_bytes());
        png.extend_from_slice(b"iCCP");
        png.extend_from_slice(chunk_data);
        png.extend_from_slice(&[0, 0, 0, 0]);

        let tiff = build_tiff_le(&[(0xA001, 3, 1)]);
        let mut exif_chunk = Vec::new();
        exif_chunk.extend_from_slice(&(tiff.len() as u32).to_be_bytes());
        exif_chunk.extend_from_slice(b"eXIf");
        exif_chunk.extend_from_slice(&tiff);
        exif_chunk.extend_from_slice(&[0, 0, 0, 0]);
        png.extend_from_slice(&exif_chunk);

        let details = get_image_details_with_grayscale(ColorType::Rgb8, "PNG", &png, false);
        assert_eq!(details.c_profile, "sRGB"); // from ICC, not EXIF
    }

    #[test]
    fn image_details_jpeg_with_real_file() {
        let img = RgbImage::from_pixel(8, 8, image::Rgb([255, 0, 0]));
        let mut jpeg_bytes = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, 80);
        encoder
            .encode(img.as_raw(), 8, 8, image::ExtendedColorType::Rgb8)
            .unwrap();

        let details = get_image_details_with_grayscale(ColorType::Rgb8, "JPEG", &jpeg_bytes, false);
        assert_eq!(details.bit_depth, "8 bits/channel");
        assert!(!details.has_alpha);
        assert_eq!(details.colorspace, "YCbCr");
        assert_eq!(details.chroma_format, Some("4:4:4".to_string()));
    }

    #[test]
    fn image_details_png() {
        let img = RgbImage::from_pixel(8, 8, image::Rgb([0, 255, 0]));
        let mut png_bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .unwrap();

        let details = get_image_details_with_grayscale(ColorType::Rgb8, "PNG", &png_bytes, false);
        assert_eq!(details.chroma_format, Some("4:4:4".to_string()));
    }

    #[test]
    fn image_details_heic() {
        let dir = tempdir().unwrap();
        let Some(path) = create_test_heic(&dir, "test.heic") else {
            println!("Skipping test: libheif encoding failed.");
            return;
        };
        let raw = std::fs::read(&path).unwrap();
        let details = get_image_details_with_grayscale(ColorType::Rgb8, "HEIC", &raw, false);
        assert_eq!(details.colorspace, "YCbCr");
        assert!(details.chroma_format.is_some());
    }
}
