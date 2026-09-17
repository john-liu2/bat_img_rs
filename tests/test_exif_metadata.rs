// Test bat_img_rs::exif::metadata
// Copyright © 2026 - Present, John Liu

mod common;

#[cfg(test)]
mod tests {
    use super::common::{
        build_tiff_le, build_tiff_with_gps, jpeg_with_exif, png_with_exif_chunk,
        webp_with_exif_chunk,
    };
    use bat_img_rs::exif::{
        extract_exif_tiff, inject_exif_into_tiff, is_png, is_tiff, parse_exif_bytes, read_exif,
        rewrite_exif_metadata, strip_all_metadata, strip_gps_from_tiff, strip_gps_metadata,
    };
    use image::RgbImage;
    use tempfile::TempDir;

    #[test]
    fn test_inject_exif_too_small() {
        let small = b"II*";
        let valid = b"II\x2A\x00\x08\x00\x00\x00\x00\x00\x00\x00";

        let result = inject_exif_into_tiff(small, valid).unwrap();

        // Output file < 8 bytes should safely abort and return unmodified
        assert_eq!(result, small);
    }

    #[test]
    fn test_inject_exif_endianness_mismatch() {
        // Little-Endian output TIFF
        let le_tiff = b"II\x2A\x00\x08\x00\x00\x00\x00\x00\x00\x00";
        // Big-Endian EXIF source
        let be_tiff = build_tiff_with_gps(0x1234);

        let result = inject_exif_into_tiff(le_tiff, &be_tiff).unwrap();

        assert!(result.len() > le_tiff.len());
        let info = parse_exif_bytes(&result).unwrap();
        assert_eq!(info.make.as_deref(), Some("Apple"));
    }

    #[test]
    fn test_inject_exif_merges_unique_tags() {
        // Construct a basic Little-Endian TIFF with 1 tag (ImageWidth 0x0100)
        let dest = vec![
            0x49, 0x49, 0x2A, 0x00, // "II", 42
            0x08, 0x00, 0x00, 0x00, // IFD0 at offset 8
            0x01, 0x00, // Count: 1 entry
            0x00, 0x01, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00,
            0x00, // Tag 0x0100, Type u32, Count 1, Value 8
            0x00, 0x00, 0x00, 0x00, // Next IFD = 0
        ];

        // Construct a source Little-Endian TIFF with 2 tags (ImageWidth 0x0100, ImageLength 0x0101)
        let source = vec![
            0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00, 0x02, 0x00, // Count: 2 entries
            0x00, 0x01, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00,
            0x00, // Tag 0x0100
            0x01, 0x01, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00,
            0x00, // Tag 0x0101
            0x00, 0x00, 0x00, 0x00,
        ];

        let grafted = inject_exif_into_tiff(&dest, &source).unwrap();

        // Output should be strictly larger since we appended the new IFD block and the missing tag (0x0101)
        assert!(grafted.len() > dest.len());

        // New IFD0 is appended exactly at the end of the original destination file size
        let new_ifd_offset =
            u32::from_le_bytes([grafted[4], grafted[5], grafted[6], grafted[7]]) as usize;
        assert_eq!(new_ifd_offset, dest.len());

        // Verify the newly minted IFD0 now correctly holds 2 entries (ImageWidth + ImageLength)
        let count = u16::from_le_bytes([grafted[new_ifd_offset], grafted[new_ifd_offset + 1]]);
        assert_eq!(count, 2);
    }

    #[test]
    fn test_rewrite_exif_metadata_for_tiff() {
        let tiff = build_tiff_with_gps(0x1234);

        // Simulates encoding step: create a bare Big-Endian ("MM") TIFF with ONLY structural tags.
        // build_tiff_with_gps produces a Big-Endian TIFF, so we must mock a Big-Endian re-encoded
        // TIFF to pass the endianness mismatch guard in inject_exif_into_tiff.
        let stripped_all = vec![
            0x4D, 0x4D, 0x00, 0x2A, // "MM\0*"
            0x00, 0x00, 0x00, 0x08, // IFD0 offset = 8
            0x00, 0x02, // Count = 2 entries
            // Tag 0x0100 (ImageWidth), Type 4 (u32), Count 1, Value 8
            0x01, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x08,
            // Tag 0x0101 (ImageLength), Type 4 (u32), Count 1, Value 8
            0x01, 0x01, 0x00, 0x04, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00,
            0x00, 0x00, // Next IFD = 0
        ];

        let source_stripped = strip_gps_metadata(&tiff).unwrap();

        let grafted = rewrite_exif_metadata(&stripped_all, &source_stripped, false).unwrap();

        assert!(is_tiff(&grafted));
        let info = parse_exif_bytes(&grafted).unwrap();

        // The Make tag from the source should now be successfully grafted
        assert_eq!(info.make.as_deref(), Some("Apple"));
        assert!(!info.gps_present);

        // The output should be strictly larger since we appended the missing metadata
        assert!(grafted.len() > stripped_all.len());
    }

    #[test]
    fn read_exif_succeeds_after_strip_gps() {
        let dir = TempDir::new().unwrap();
        let input_path = dir.path().join("input_with_gps.heic");
        let stripped_path = dir.path().join("stripped_gps.heic");

        let img =
            image::DynamicImage::ImageRgb8(RgbImage::from_pixel(8, 8, image::Rgb([10, 20, 30])));
        let tiff = build_tiff_with_gps(0x1234);
        bat_img_rs::heic::encode(
            &img,
            &input_path,
            libheif_rs::CompressionFormat::Hevc,
            Some(80),
            Some(&tiff),
            None,
        )
        .unwrap();

        let raw_bytes = std::fs::read(&input_path).unwrap();
        let stripped_bytes = strip_gps_metadata(&raw_bytes).expect("GPS stripping failed");
        std::fs::write(&stripped_path, &stripped_bytes).unwrap();

        let exif_info = read_exif(&stripped_path)
            .expect("read_exif must return valid EXIF metadata after strip_gps");

        assert_eq!(exif_info.make.as_deref(), Some("Apple"));
        assert!(!exif_info.gps_present);
    }

    #[test]
    fn png_strip_gps_metadata_preserves_chunk_structure() {
        let tiff = build_tiff_with_gps(0x9ABC);
        let png = png_with_exif_chunk(&tiff);

        let stripped = strip_gps_metadata(&png).unwrap();

        assert!(is_png(&stripped));
        assert!(stripped.windows(4).any(|w| w == b"eXIf"));

        let gps_bytes = 0x9ABCu32.to_le_bytes();
        assert!(!stripped.windows(4).any(|w| w == gps_bytes));
    }

    #[test]
    fn webp_strip_gps_metadata_zeroes_gps_ifd() {
        let tiff = build_tiff_with_gps(0xDEF0);
        let webp = webp_with_exif_chunk(&tiff);

        let stripped = strip_gps_metadata(&webp).unwrap();

        assert!(stripped.starts_with(b"RIFF"));
        assert!(stripped.windows(4).any(|w| w == b"EXIF"));

        let gps_bytes = 0xDEF0u32.to_le_bytes();
        assert!(!stripped.windows(4).any(|w| w == gps_bytes));
    }

    #[test]
    fn tiff_strip_gps_metadata_zeroes_ifd_tag() {
        let tiff = build_tiff_with_gps(0x4321);

        let stripped = strip_gps_metadata(&tiff).unwrap();

        assert!(is_tiff(&stripped));
        assert_eq!(stripped.len(), tiff.len());

        let gps_bytes = 0x4321u32.to_le_bytes();
        assert!(!stripped.windows(4).any(|w| w == gps_bytes));
    }

    #[test]
    fn tiff_strip_all_metadata_removes_pointers() {
        let tiff = build_tiff_with_gps(0x7777);

        let stripped = strip_all_metadata(&tiff).unwrap();

        assert!(is_tiff(&stripped));
        let gps_bytes = 0x7777u32.to_le_bytes();
        assert!(!stripped.windows(4).any(|w| w == gps_bytes));
    }

    #[test]
    fn strip_all_removes_app1_keeps_soi() {
        let tiff = build_tiff_le(&[(0x0112, 3, 1)]);
        let jpeg = jpeg_with_exif(&tiff);
        let stripped = strip_all_metadata(&jpeg).unwrap();

        assert!(stripped.starts_with(&[0xFF, 0xD8]));
        assert!(!stripped.windows(2).any(|w| w == [0xFF, 0xE1]));
    }

    #[test]
    fn strip_all_non_jpeg_passthrough() {
        let data = b"\x89PNG\r\n\x1a\nsome_data";
        let result = strip_all_metadata(data).unwrap();
        assert_eq!(result, data.as_ref());
    }

    #[test]
    fn strip_all_idempotent() {
        let tiff = build_tiff_le(&[(0x0112, 3, 6)]);
        let jpeg = jpeg_with_exif(&tiff);
        let once = strip_all_metadata(&jpeg).unwrap();
        let twice = strip_all_metadata(&once).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn strip_all_preserves_length_or_shrinks() {
        let tiff = build_tiff_le(&[(0x0112, 3, 1)]);
        let jpeg = jpeg_with_exif(&tiff);
        let stripped = strip_all_metadata(&jpeg).unwrap();
        assert!(stripped.len() <= jpeg.len());
    }

    #[test]
    fn graft_exif_preserves_app1_on_real_jpeg_encode() {
        let mut img = RgbImage::new(8, 8);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgb([1, 2, 3]);
        }
        let mut encoded = Vec::new();
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 90);
        enc.encode(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();

        let tiff = build_tiff_with_gps(0x1234);
        let jpeg = jpeg_with_exif(&tiff);
        let stripped = strip_gps_metadata(&jpeg).unwrap();

        let grafted = rewrite_exif_metadata(&encoded, &stripped, false).unwrap();
        assert!(grafted.windows(2).any(|w| w == [0xFF, 0xE1]));
    }

    #[test]
    fn graft_exif_preserves_app1_after_reencode() {
        let tiff = build_tiff_with_gps(0x1234);
        let jpeg = jpeg_with_exif(&tiff);
        let stripped = strip_gps_metadata(&jpeg).unwrap();
        assert!(extract_exif_tiff(&stripped).is_some());

        let encoded = strip_all_metadata(&jpeg).unwrap();
        assert!(!encoded.windows(2).any(|w| w == [0xFF, 0xE1]));

        let grafted = rewrite_exif_metadata(&encoded, &stripped, false).unwrap();
        assert!(grafted.windows(2).any(|w| w == [0xFF, 0xE1]));
        assert!(!grafted.windows(4).any(|w| w == 0x1234u32.to_le_bytes()));
    }

    #[test]
    fn strip_gps_zeroes_gps_ifd_pointer() {
        let tiff = build_tiff_with_gps(0x1234);
        let jpeg = jpeg_with_exif(&tiff);
        let stripped = strip_gps_metadata(&jpeg).unwrap();

        assert!(stripped.starts_with(&[0xFF, 0xD8]));

        let gps_bytes = 0x1234u32.to_le_bytes();
        let found = stripped.windows(4).any(|w| w == gps_bytes);
        assert!(!found);
    }

    #[test]
    fn strip_gps_no_gps_is_noop() {
        let tiff = build_tiff_le(&[(0x0112, 3, 1)]);
        let jpeg = jpeg_with_exif(&tiff);
        let stripped = strip_gps_metadata(&jpeg).unwrap();
        assert!(stripped.starts_with(&[0xFF, 0xD8]));
        assert!(stripped.len() <= jpeg.len());
    }

    #[test]
    fn strip_gps_non_jpeg_passthrough() {
        let data = b"\x89PNG\r\n\x1a\nsome_data";
        let result = strip_gps_metadata(data).unwrap();
        assert_eq!(result, data.as_ref());
    }

    #[test]
    fn strip_gps_result_is_valid_jpeg_header() {
        let tiff = build_tiff_with_gps(0xFF00);
        let jpeg = jpeg_with_exif(&tiff);
        let stripped = strip_gps_metadata(&jpeg).unwrap();
        assert!(stripped.starts_with(&[0xFF, 0xD8]));
    }

    #[test]
    fn strip_png_exif_metadata_and_save_file() {
        let tmp = TempDir::new().unwrap();
        let output = tmp.path().join("output.png");
        let png = png_with_exif_chunk(b"fake_exif_data");

        let stripped = strip_all_metadata(&png).unwrap();
        std::fs::write(&output, &stripped).unwrap();
        let saved = std::fs::read(&output).unwrap();

        assert!(is_png(&saved));
        assert!(!saved.windows(4).any(|x| x == b"eXIf"));
    }

    #[test]
    fn strip_webp_exif_metadata_and_save_file() {
        let tmp = TempDir::new().unwrap();
        let output = tmp.path().join("output.webp");
        let webp = webp_with_exif_chunk(b"fake_exif");

        let stripped = strip_all_metadata(&webp).unwrap();
        std::fs::write(&output, &stripped).unwrap();
        let saved = std::fs::read(&output).unwrap();

        assert!(saved.starts_with(b"RIFF"));
        assert!(!saved.windows(4).any(|x| x == b"EXIF"));
    }

    #[test]
    fn strip_tiff_gps_metadata_and_save_file() {
        let tmp = TempDir::new().unwrap();
        let output = tmp.path().join("output.tiff");

        let tiff = build_tiff_le(&[(0x0112, 3, 6), (0x8825, 4, 1234)]);

        let stripped = strip_all_metadata(&tiff).unwrap();
        std::fs::write(&output, &stripped).unwrap();
        let saved = std::fs::read(&output).unwrap();
        assert!(is_tiff(&saved));
    }

    #[test]
    fn test_strip_gps_from_tiff() {
        let tiff = build_tiff_with_gps(0x1234);
        let stripped = strip_gps_from_tiff(&tiff).unwrap();

        assert!(!stripped.windows(4).any(|w| w == 0x1234u32.to_le_bytes()));
        let info = parse_exif_bytes(&stripped).unwrap();
        assert_eq!(info.make, Some("Apple".to_string()));
        assert!(!info.gps_present);
    }

    #[test]
    fn test_rewrite_exif_metadata() {
        let tiff = build_tiff_with_gps(0x1234);
        let jpeg = jpeg_with_exif(&tiff);

        let stripped_all = strip_all_metadata(&jpeg).unwrap();
        let source_stripped = strip_gps_metadata(&jpeg).unwrap();

        let grafted = rewrite_exif_metadata(&stripped_all, &source_stripped, false).unwrap();

        assert!(grafted.windows(2).any(|w| w == [0xFF, 0xE1]));
        assert!(!grafted.windows(4).any(|w| w == 0x1234u32.to_le_bytes()));
    }
}
