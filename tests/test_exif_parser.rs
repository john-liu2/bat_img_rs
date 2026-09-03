/// Test bat_img_rs::exif::parser
mod common;

#[cfg(test)]
mod tests {
    use super::common::{build_tiff_be, build_tiff_le, jpeg_with_exif};
    use bat_img_rs::exif::{parse_exif_bytes, read_orientation};

    // Temp local manual debug
    // #[test]
    // fn debug_read_exif_manual() {
    //     use bat_img_rs::exif::read_exif
    //     // Get home directory from environment variable
    //     let home = std::env::var("HOME").expect("HOME not set");
    //     let path = std::path::Path::new(&home).join("Downloads").join("IMG_0496.HEIC");
    //     assert!(path.exists(), "File does not exist at: {:?}", path);
    //     let result = read_exif(&path);
    //     println!("EXIF Info: {:?}", result);

    //     assert!(result.is_some(), "Failed to read EXIF from IMG_0496.HEIC");
    // }

    #[test]
    fn test_parse_exif_bytes() {
        let valid_tiff = build_tiff_le(&[(0x0112, 3, 1)]);
        let result = parse_exif_bytes(&valid_tiff);
        assert!(result.is_some());
        // Invalid data
        assert!(parse_exif_bytes(b"invalid").is_none());
    }

    // Formatting helper tests (EXIF display values)
    #[test]
    fn format_make_and_model_strips_quotes() {
        let raw_make = "\"Apple\"";
        let raw_model = "\"iPhone 16 Pro Max\"";
        assert_eq!(raw_make.trim_matches('"'), "Apple");
        assert_eq!(raw_model.trim_matches('"'), "iPhone 16 Pro Max");
    }

    #[test]
    fn format_exposure_time_strips_double_seconds() {
        let raw = "1/348 s";
        let clean = raw.trim().trim_end_matches('s').trim();
        assert_eq!(format!("{clean} s"), "1/348 s");
    }

    #[test]
    fn format_aperture_normalizes_f_number_and_floats() {
        // Double prefix + long float
        let raw = "f/f/1.7799999713880652";
        let clean_f = raw
            .trim()
            .trim_start_matches("f/")
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
        assert_eq!(format!("f/{formatted_f}"), "f/1.78");

        // Single prefix
        let raw2 = "f/2.8000";
        let clean_f2 = raw2
            .trim()
            .trim_start_matches("f/")
            .trim_start_matches('f')
            .trim();
        let formatted_f2 = if let Ok(val) = clean_f2.parse::<f64>() {
            format!("{:.2}", val)
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        } else {
            clean_f2.to_string()
        };
        assert_eq!(format!("f/{formatted_f2}"), "f/2.8");
    }

    #[test]
    fn format_focal_length_rounds_float_and_normalizes_mm() {
        let raw = "6.764999866370901 mm";
        let clean_fl = raw.trim().trim_end_matches("mm").trim();
        let formatted_fl = if let Ok(val) = clean_fl.parse::<f64>() {
            format!("{:.2} mm", val)
        } else {
            format!("{} mm", clean_fl)
        };
        assert_eq!(formatted_fl, "6.76 mm");
    }
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
        let tiff = build_tiff_le(&[(0x010F, 2, 0)]);
        let jpeg = jpeg_with_exif(&tiff);
        assert_eq!(read_orientation(&jpeg), None);
    }

    #[test]
    fn orientation_non_jpeg_returns_none() {
        let png = b"\x89PNG\r\n\x1a\n";
        assert_eq!(read_orientation(png), None);
    }

    #[test]
    fn orientation_empty_bytes_returns_none() {
        assert_eq!(read_orientation(&[]), None);
    }

    #[test]
    fn orientation_jpeg_without_exif_returns_none() {
        let mut jpeg = vec![0xFF, 0xD8];
        let app0_payload = b"JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00";
        let seg_len = (app0_payload.len() + 2) as u16;
        jpeg.push(0xFF);
        jpeg.push(0xE0);
        jpeg.extend_from_slice(&seg_len.to_be_bytes());
        jpeg.extend_from_slice(app0_payload);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        assert_eq!(read_orientation(&jpeg), None);
    }
}
