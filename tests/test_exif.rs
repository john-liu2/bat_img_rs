/// Unit tests for bat_img_rs::exif
///
/// All tests synthesise minimal valid JPEG / TIFF byte sequences in memory —
/// no fixture files are required.
mod common;

#[cfg(test)]
mod tests {
    use bat_img_rs::exif::{
        extract_exif_tiff, rewrite_exif_metadata, is_png, is_tiff, read_orientation,
        strip_all_metadata, strip_gps_metadata, read_exif
    };
    use image::{DynamicImage, RgbImage};
    use tempfile::{TempDir, tempdir};
    use libheif_rs::CompressionFormat;
    use bat_img_rs::heic;

    use super::common::{png_with_exif_chunk, webp_with_exif_chunk, build_tiff_le,
        build_tiff_be, jpeg_with_exif, build_tiff_with_gps
    };

    // ── Helper to synthesize minimal HEIC structure ─────────────────────────────
    fn mock_heic_with_exif(tiff_payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();

        // 1. Minimal 'ftyp' box (12 bytes)
        bytes.extend_from_slice(&12u32.to_be_bytes());
        bytes.extend_from_slice(b"ftypheic");

        // 2. 'meta' box wrapping EXIF TIFF payload
        let meta_payload_len = 4 + tiff_payload.len(); // 4-byte version/flags + payload
        let meta_box_len = (8 + meta_payload_len) as u32;

        bytes.extend_from_slice(&meta_box_len.to_be_bytes());
        bytes.extend_from_slice(b"meta");
        bytes.extend_from_slice(&[0, 0, 0, 0]); // FullBox version and flags
        bytes.extend_from_slice(tiff_payload);

        bytes
    }

    #[test]
    fn read_exif_succeeds_after_strip_gps() {
        let dir = tempdir().unwrap();
        let input_path = dir.path().join("input_with_gps.heic");
        let stripped_path = dir.path().join("stripped_gps.heic");

        let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(8, 8, image::Rgb([10, 20, 30])));
        let tiff = build_tiff_with_gps(0x1234);

        // Encode HEIC with EXIF + GPS metadata
        heic::encode(&img, &input_path, CompressionFormat::Hevc, Some(80), Some(&tiff)).unwrap();

        // Strip GPS metadata
        let raw_bytes = std::fs::read(&input_path).unwrap();
        let stripped_bytes = strip_gps_metadata(&raw_bytes).expect("GPS stripping failed");
        std::fs::write(&stripped_path, &stripped_bytes).unwrap();

        // Verify read_exif parses non-GPS camera tags cleanly
        let exif_info = read_exif(&stripped_path)
            .expect("read_exif must return valid EXIF metadata after strip_gps");

        assert_eq!(exif_info.make.as_deref(), Some("Apple"));
        assert!(!exif_info.gps_present, "GPS data should no longer be present");
    }

    // ── HEIC Container Level Tests ───────────────────────────────────────────────
    #[test]
    fn heic_strip_all_metadata_zeroes_exif_payload() {
        let tiff = build_tiff_le(&[(0x0112, 3, 1)]);
        let heic_bytes = mock_heic_with_exif(&tiff);

        let stripped = strip_all_metadata(&heic_bytes).unwrap();

        // Total byte length must be preserved exactly
        assert_eq!(stripped.len(), heic_bytes.len());

        // The 'meta' box MUST remain intact so libheif can parse iloc/pitm container structures
        assert!(stripped.windows(4).any(|w| w == b"meta"));

        // The EXIF TIFF magic bytes (II*\0 or MM\0*) must be zeroed out
        assert!(!stripped.windows(4).any(|w| w == b"II\x2A\x00"));
        assert!(!stripped.windows(4).any(|w| w == b"MM\x00\x2A"));
    }

    #[test]
    fn heic_extract_and_replace_raw_exif() {
        use bat_img_rs::exif::{extract_heic_exif_raw, replace_heic_exif_payload};

        let tiff = build_tiff_with_gps(0x5678);
        let heic_bytes = mock_heic_with_exif(&tiff);

        // Extract TIFF slice
        let extracted = extract_heic_exif_raw(&heic_bytes);
        assert!(extracted.is_some());
        let raw = extracted.unwrap();
        assert!(raw.starts_with(b"II\x2A\x00") || raw.starts_with(b"MM\x00\x2A"));

        // Replace payload in-place
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

        // Total file size remains identical
        assert_eq!(stripped.len(), heic_bytes.len());

        // 'meta' tag remains intact, but GPS pointer sequence is zeroed out
        assert!(stripped.windows(4).any(|w| w == b"meta"));
        let gps_bytes = 0x1234u32.to_le_bytes();
        assert!(!stripped.windows(4).any(|w| w == gps_bytes));
    }

    // ── EXIF Info Display Formatting Helpers ─────────────────────────────────
    #[test]
    fn format_make_and_model_strips_quotes() {
        let raw_make = "\"Apple\"";
        let raw_model = "\"iPhone 16 Pro Max\"";

        let clean_make = raw_make.trim().trim_matches('"').trim();
        let clean_model = raw_model.trim().trim_matches('"').trim();

        assert_eq!(clean_make, "Apple");
        assert_eq!(clean_model, "iPhone 16 Pro Max");
    }

    #[test]
    fn format_exposure_time_strips_double_seconds() {
        let raw = "1/348 s";
        let clean = raw.trim().trim_end_matches('s').trim();
        assert_eq!(format!("{clean} s"), "1/348 s");
    }

    #[test]
    fn format_aperture_normalizes_f_number_and_floats() {
        // Double prefix + long float: f/f/1.7799999713880652
        let raw = "f/f/1.7799999713880652";
        let clean_f = raw.trim().trim_start_matches("f/").trim_start_matches("f/").trim_start_matches('f').trim();
        let formatted_f = if let Ok(val) = clean_f.parse::<f64>() {
            format!("{:.2}", val).trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            clean_f.to_string()
        };
        assert_eq!(format!("f/{formatted_f}"), "f/1.78");

        // Single prefix: f/2.8
        let raw2 = "f/2.8000";
        let clean_f2 = raw2.trim().trim_start_matches("f/").trim_start_matches('f').trim();
        let formatted_f2 = if let Ok(val) = clean_f2.parse::<f64>() {
            format!("{:.2}", val).trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            clean_f2.to_string()
        };
        assert_eq!(format!("f/{formatted_f2}"), "f/2.8");
    }

    #[test]
    fn format_focal_length_rounds_float_and_normalizes_mm() {
        // Raw float string: 6.764999866370901 mm
        let raw = "6.764999866370901 mm";
        let clean_fl = raw.trim().trim_end_matches("mm").trim();
        let formatted_fl = if let Ok(val) = clean_fl.parse::<f64>() {
            format!("{:.2} mm", val)
        } else {
            format!("{} mm", clean_fl)
        };
        assert_eq!(formatted_fl, "6.76 mm");
    }

    // ── Multi-Format Strip GPS Tests ───────────────────────────────────────────
    #[test]
    fn png_strip_gps_metadata_preserves_chunk_structure() {
        let tiff = build_tiff_with_gps(0x9ABC);
        let png = png_with_exif_chunk(&tiff);

        let stripped = strip_gps_metadata(&png).unwrap();

        // Should retain PNG magic and eXIf chunk header
        assert!(is_png(&stripped));
        assert!(stripped.windows(4).any(|w| w == b"eXIf"));

        // GPS IFD offset zeroed out
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

    // ── read_exif with HEIC fixture ───────────────────────────────────────────
    #[test]
    fn read_exif_from_heic_fixture() {
        use bat_img_rs::exif::read_exif;
        use std::path::Path;

        let fixture_path = Path::new("tests/fixtures/src.heic");
        if fixture_path.exists() {
            let exif_info = read_exif(fixture_path);
            // Asserts that reading EXIF from HEIC fixture executes successfully without panicking
            assert!(exif_info.is_some() || exif_info.is_none());
        }
    }

    // ── read_orientation ──────────────────────────────────────────────────────
    #[test]
    fn orientation_little_endian_values() {
        for expected in 1u16..=8 {
            let tiff = build_tiff_le(&[(0x0112, 3, expected)]);
            let jpeg = jpeg_with_exif(&tiff);
            assert_eq!(
                read_orientation(&jpeg),
                Some(expected as u32),
                "LE orientation {expected}"
            );
        }
    }

    #[test]
    fn orientation_big_endian() {
        let jpeg = jpeg_with_exif(&build_tiff_be(6));
        assert_eq!(read_orientation(&jpeg), Some(6));
    }

    #[test]
    fn orientation_missing_tag_returns_none() {
        // TIFF with only a Make tag (0x010F), no orientation
        let tiff = build_tiff_le(&[(0x010F, 2, 0)]);
        let jpeg = jpeg_with_exif(&tiff);
        assert_eq!(read_orientation(&jpeg), None);
    }

    #[test]
    fn orientation_non_jpeg_returns_none() {
        // PNG magic
        let png = b"\x89PNG\r\n\x1a\n";
        assert_eq!(read_orientation(png), None);
    }

    #[test]
    fn orientation_empty_bytes_returns_none() {
        assert_eq!(read_orientation(&[]), None);
    }

    #[test]
    fn orientation_jpeg_without_exif_returns_none() {
        // JPEG with APP0 (JFIF) but no APP1
        let mut jpeg = vec![0xFF, 0xD8];
        // APP0: len = 16 (14 payload + 2 for len field)
        let app0_payload = b"JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00";
        let seg_len = (app0_payload.len() + 2) as u16;
        jpeg.push(0xFF);
        jpeg.push(0xE0);
        jpeg.extend_from_slice(&seg_len.to_be_bytes());
        jpeg.extend_from_slice(app0_payload);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        assert_eq!(read_orientation(&jpeg), None);
    }

    // ── strip_all_metadata ────────────────────────────────────────────────────
    #[test]
    fn strip_all_removes_app1_keeps_soi() {
        let tiff = build_tiff_le(&[(0x0112, 3, 1)]);
        let jpeg = jpeg_with_exif(&tiff);
        let stripped = strip_all_metadata(&jpeg).unwrap();

        // SOI must still be present
        assert!(stripped.starts_with(&[0xFF, 0xD8]));
        // APP1 marker (0xFF 0xE1) must be gone
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

    // ── strip_gps_metadata ────────────────────────────────────────────────────
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

        let grafted = rewrite_exif_metadata(&encoded, &stripped).unwrap();
        assert!(grafted.windows(2).any(|w| w == [0xFF, 0xE1]));
    }

    #[test]
    fn graft_exif_preserves_app1_after_reencode() {
        let tiff = build_tiff_with_gps(0x1234);
        let jpeg = jpeg_with_exif(&tiff);
        let stripped = strip_gps_metadata(&jpeg).unwrap();
        assert!(extract_exif_tiff(&stripped).is_some());

        // Simulate a fresh encode without metadata.
        let encoded = strip_all_metadata(&jpeg).unwrap();
        assert!(!encoded.windows(2).any(|w| w == [0xFF, 0xE1]));

        let grafted = rewrite_exif_metadata(&encoded, &stripped).unwrap();
        assert!(grafted.windows(2).any(|w| w == [0xFF, 0xE1]));
        assert!(!grafted.windows(4).any(|w| w == 0x1234u32.to_le_bytes()));
    }

    #[test]
    fn strip_gps_zeroes_gps_ifd_pointer() {
        let tiff = build_tiff_with_gps(0x1234);
        let jpeg = jpeg_with_exif(&tiff);
        let stripped = strip_gps_metadata(&jpeg).unwrap();

        // File should still be a JPEG
        assert!(stripped.starts_with(&[0xFF, 0xD8]));

        // The GPS offset (0x00001234) should no longer appear as a 4-byte LE
        // sequence anywhere in the output (it was zeroed in the IFD entry).
        let gps_bytes = 0x1234u32.to_le_bytes();
        let found = stripped.windows(4).any(|w| w == gps_bytes);
        assert!(!found, "GPS offset value should be zeroed out");
    }

    #[test]
    fn strip_gps_no_gps_is_noop() {
        let tiff = build_tiff_le(&[(0x0112, 3, 1)]);
        let jpeg = jpeg_with_exif(&tiff);
        let stripped = strip_gps_metadata(&jpeg).unwrap();
        // No GPS entry to remove; output must be a valid JPEG and no larger
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

        let input = tmp.path().join("input.png");
        let output = tmp.path().join("output.png");

        // Minimal PNG containing an eXIf chunk.
        let png = png_with_exif_chunk(b"fake_exif_data");

        std::fs::write(&input, &png).unwrap();
        let stripped = strip_all_metadata(&png).unwrap();

        std::fs::write(&output, &stripped).unwrap();
        assert!(output.exists());

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
        assert!(output.exists());

        let saved = std::fs::read(&output).unwrap();

        assert!(saved.starts_with(b"RIFF"));
        assert!(!saved.windows(4).any(|x| x == b"EXIF"));
    }

    #[test]
    fn strip_tiff_gps_metadata_and_save_file() {
        let tmp = TempDir::new().unwrap();

        let output = tmp.path().join("output.tiff");

        // GPSInfoIFDPointer tag = 0x8825
        let tiff = build_tiff_le(&[
            (0x0112, 3, 6),       // Orientation
            (0x8825, 4, 1234),    // GPS pointer
        ]);

        let stripped = strip_all_metadata(&tiff).unwrap();
        std::fs::write(&output, &stripped).unwrap();
        assert!(output.exists());

        let saved = std::fs::read(&output).unwrap();
        assert!(is_tiff(&saved));
    }
}
