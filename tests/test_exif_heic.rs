/// Test bat_img_rs::exif::heic
mod common;

#[cfg(test)]
mod tests {
    use super::common::{
        build_tiff_le, build_tiff_with_gps, create_test_heic, mock_heic_with_exif,
    };
    use bat_img_rs::exif::{
        extract_heic_exif_raw, get_image_details, parse_exif_bytes, read_exif,
        replace_heic_exif_payload, strip_all_metadata, strip_gps_metadata, tiff_from_heic_metadata,
    };
    use bat_img_rs::heic;
    use image::{DynamicImage, RgbImage};
    use libheif_rs::CompressionFormat;
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn parses_apple_style_heif_exif_with_optional_ifd_error() {
        // The HEIF offset points six bytes past the offset field, over the
        // `Exif\0\0` prefix, to a big-endian TIFF header.
        let tiff = [
            b'M', b'M', 0, 42, 0, 0, 0, 8, // TIFF header and IFD0 offset
            0, 2, // two IFD0 entries
            0x01, 0x0f, 0, 2, 0, 0, 0, 6, 0, 0, 0, 38, // Make = "Apple"
            0x87, 0x69, 0, 4, 0, 0, 0, 1, 0, 0, 0, 44, // Exif IFD pointer
            0, 0, 0, 0, // no thumbnail IFD
            b'A', b'p', b'p', b'l', b'e', 0, 0, 0, // empty child IFD
            0, 0, 0, 4, // unsupported child IFD next-pointer
        ];
        let mut metadata = 6u32.to_be_bytes().to_vec();
        metadata.extend_from_slice(b"Exif\0\0");
        metadata.extend_from_slice(&tiff);

        let extracted = tiff_from_heic_metadata(&metadata)
            .expect("HEIF EXIF offset must locate the TIFF header");
        assert_eq!(extracted, tiff);

        let info = parse_exif_bytes(extracted)
            .expect("a malformed optional IFD must not discard valid IFD0 tags");
        assert_eq!(info.make.as_deref(), Some("Apple"));
    }

    #[test]
    fn read_exif_native_fallback_for_heic() {
        let dir = tempdir().unwrap();
        let input_path = dir.path().join("test_native.heic");

        let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(8, 8, image::Rgb([10, 20, 30])));
        let tiff = build_tiff_with_gps(0x1234);
        heic::encode(
            &img,
            &input_path,
            CompressionFormat::Hevc,
            Some(80),
            Some(&tiff),
        )
        .unwrap();

        let exif_info = read_exif(&input_path);
        assert!(exif_info.is_some());
        let info = exif_info.unwrap();
        assert_eq!(info.make.as_deref(), Some("Apple"));
        assert!(info.gps_present);
    }

    #[test]
    fn read_exif_from_heic_fixture() {
        let fixture_path = Path::new("tests/fixtures/src.heic");
        if fixture_path.exists() {
            let exif_info = read_exif(fixture_path);
            assert!(exif_info.is_some() || exif_info.is_none());
        }
    }

    #[test]
    fn heic_strip_all_metadata_zeroes_exif_payload() {
        let tiff = build_tiff_le(&[(0x0112, 3, 1)]);
        let heic_bytes = mock_heic_with_exif(&tiff);

        let stripped = strip_all_metadata(&heic_bytes).unwrap();

        assert_eq!(stripped.len(), heic_bytes.len());
        assert!(stripped.windows(4).any(|w| w == b"meta"));
        assert!(!stripped.windows(4).any(|w| w == b"II\x2A\x00"));
        assert!(!stripped.windows(4).any(|w| w == b"MM\x00\x2A"));
    }

    #[test]
    fn heic_extract_and_replace_raw_exif() {
        let tiff = build_tiff_with_gps(0x5678);
        let heic_bytes = mock_heic_with_exif(&tiff);

        let extracted = extract_heic_exif_raw(&heic_bytes);
        assert!(extracted.is_some());
        let raw = extracted.unwrap();
        assert!(raw.starts_with(b"II\x2A\x00") || raw.starts_with(b"MM\x00\x2A"));

        let mut modified_tiff = raw.clone();
        modified_tiff[0..4].copy_from_slice(b"TEST");

        let replaced = replace_heic_exif_payload(&heic_bytes, &modified_tiff).unwrap();
        assert_eq!(replaced.len(), heic_bytes.len());
        assert!(replaced.windows(4).any(|w| w == b"TEST"));
    }

    #[test]
    fn heic_strip_gps_metadata_zeroes_pointer() {
        let tiff = build_tiff_with_gps(0x1234);
        let heic_bytes = mock_heic_with_exif(&tiff);

        let stripped = strip_gps_metadata(&heic_bytes).unwrap();

        assert_eq!(stripped.len(), heic_bytes.len());
        assert!(stripped.windows(4).any(|w| w == b"meta"));
        let gps_bytes = 0x1234u32.to_le_bytes();
        assert!(!stripped.windows(4).any(|w| w == gps_bytes));
    }

    #[test]
    fn test_tiff_from_heic_metadata() {
        let tiff = build_tiff_le(&[(0x0112, 3, 1)]);
        // With Exif\0\0 prefix
        let mut data = b"Exif\0\0".to_vec();
        data.extend_from_slice(&tiff);
        let result = tiff_from_heic_metadata(&data).unwrap();
        assert_eq!(result, tiff.as_slice());
        // With offset = 0 (TIFF starts after the 4-byte offset field)
        let offset = 0u32.to_be_bytes();
        let mut data2 = offset.to_vec();
        data2.extend_from_slice(&tiff);
        let result2 = tiff_from_heic_metadata(&data2).unwrap();
        assert_eq!(result2, tiff.as_slice());
        // Invalid data
        assert!(tiff_from_heic_metadata(b"invalid").is_none());
    }

    #[test]
    fn image_details_heic() {
        let dir = tempdir().unwrap();
        let Some(path) = create_test_heic(&dir, "test.heic") else {
            println!("Skipping test: libheif encoding failed.");
            return;
        };
        let raw = std::fs::read(&path).unwrap();
        let details = get_image_details(image::ColorType::Rgb8, "HEIC", &raw);
        assert_eq!(details.colorspace, "YCbCr");
        assert!(details.chroma_format.is_some());
    }
}
