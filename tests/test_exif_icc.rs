/// Test bat_img_rs::exif::icc
mod common;

#[cfg(test)]
mod tests {
    use super::common::mock_icc_profile;
    use bat_img_rs::exif::{get_icc_profile_name, get_image_details};
    use image::ColorType;
    use std::path::Path;

    #[test]
    fn test_extract_icc_profile_name_jpeg() {
        let icc = mock_icc_profile("Display P3");
        let mut jpeg = vec![0xFF, 0xD8];

        let app2_len = (14 + icc.len() + 2) as u16;
        jpeg.extend_from_slice(&[0xFF, 0xE2]);
        jpeg.extend_from_slice(&app2_len.to_be_bytes());
        jpeg.extend_from_slice(b"ICC_PROFILE\0");
        jpeg.extend_from_slice(&[1, 1]);
        jpeg.extend_from_slice(&icc);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);

        let details = get_image_details(ColorType::Rgb8, "JPEG", &jpeg);
        assert_eq!(details.c_profile, "Display P3");
    }

    #[test]
    fn test_extract_icc_profile_name_png() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let chunk_data = b"sRGB\0\0fake_compressed_payload";
        png.extend_from_slice(&(chunk_data.len() as u32).to_be_bytes());
        png.extend_from_slice(b"iCCP");
        png.extend_from_slice(chunk_data);
        png.extend_from_slice(&[0, 0, 0, 0]);

        let details = get_image_details(ColorType::Rgb8, "PNG", &png);
        assert_eq!(details.c_profile, "sRGB");
    }

    #[test]
    fn test_extract_icc_profile_name_tiff() {
        let icc = mock_icc_profile("ProPhoto RGB");
        let mut tiff = b"II\x2a\x00\x08\x00\x00\x00".to_vec();

        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&34675u16.to_le_bytes());
        tiff.extend_from_slice(&7u16.to_le_bytes());
        tiff.extend_from_slice(&(icc.len() as u32).to_le_bytes());

        let payload_offset = 8 + 2 + 12 + 4;
        tiff.extend_from_slice(&(payload_offset as u32).to_le_bytes());
        tiff.extend_from_slice(&[0, 0, 0, 0]);
        tiff.extend_from_slice(&icc);

        let details = get_image_details(ColorType::Rgb8, "TIFF", &tiff);
        assert_eq!(details.c_profile, "ProPhoto RGB");
    }

    #[test]
    fn test_extract_icc_profile_name_heic() {
        let icc = mock_icc_profile("Adobe RGB (1998)");
        let mut heic = Vec::new();

        let box_size = (12 + icc.len()) as u32;
        heic.extend_from_slice(&box_size.to_be_bytes());
        heic.extend_from_slice(b"colr");
        heic.extend_from_slice(b"prof");
        heic.extend_from_slice(&icc);

        let details = get_image_details(ColorType::Rgb8, "HEIC", &heic);
        assert_eq!(details.c_profile, "Adobe RGB (1998)");
    }

    #[test]
    fn fixture_heic_extract_icc_profile_name() {
        let fixture = Path::new("tests/fixtures/Cartoon.heic");
        assert!(fixture.exists());
        let raw_bytes = std::fs::read(fixture).unwrap();
        let details = get_image_details(ColorType::Rgb8, "HEIC", &raw_bytes);
        assert_eq!(details.c_profile, "LG UltraFine");
    }

    #[test]
    fn fixture_jpeg_extract_icc_profile_name() {
        let fixture = Path::new("tests/fixtures/IMG_4412.jpeg");
        assert!(fixture.exists());
        let raw_bytes = std::fs::read(fixture).unwrap();
        let details = get_image_details(ColorType::Rgb8, "JPEG", &raw_bytes);
        assert_eq!(details.c_profile, "sRGB IEC61966-2.1");
    }

    #[test]
    fn fixture_png_extract_icc_profile_name() {
        let fixture = Path::new("tests/fixtures/Cute.png");
        assert!(fixture.exists());
        let raw_bytes = std::fs::read(fixture).unwrap();
        let details = get_image_details(image::ColorType::Rgb8, "PNG", &raw_bytes);
        assert_eq!(details.c_profile, "LG UltraFine");
    }

    #[test]
    fn test_get_icc_profile_name() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let chunk_data = b"sRGB\0\0fake_compressed_payload";
        png.extend_from_slice(&(chunk_data.len() as u32).to_be_bytes());
        png.extend_from_slice(b"iCCP");
        png.extend_from_slice(chunk_data);
        png.extend_from_slice(&[0, 0, 0, 0]);

        let name = get_icc_profile_name("PNG", &png).unwrap();
        assert_eq!(name, "sRGB");
        assert!(get_icc_profile_name("BMP", b"").is_none());
    }
}
