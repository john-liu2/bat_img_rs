/// Test bat_img_rs::exif::container
mod common;

#[cfg(test)]
mod tests {
    use super::common::{build_tiff_le, jpeg_with_exif, png_with_exif_chunk, webp_with_exif_chunk};
    use bat_img_rs::exif::{extract_exif_tiff, is_jpeg, is_png, is_tiff, is_webp};

    #[test]
    fn test_is_jpeg() {
        let jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
        assert!(is_jpeg(&jpeg));
        assert!(!is_jpeg(b"PNG"));
    }

    #[test]
    fn test_is_png() {
        let png = b"\x89PNG\r\n\x1a\n";
        assert!(is_png(png));
        assert!(!is_png(b"JPEG"));
    }

    #[test]
    fn test_is_tiff() {
        let tiff_le = b"II\x2A\x00";
        let tiff_be = b"MM\x00\x2A";
        assert!(is_tiff(tiff_le));
        assert!(is_tiff(tiff_be));
        assert!(!is_tiff(b"TIFF"));
    }

    #[test]
    fn test_is_webp() {
        let webp = b"RIFF....WEBP"; // minimal valid RIFF/WEBP
        assert!(is_webp(webp));
        assert!(!is_webp(b"WEBP"));
    }

    #[test]
    fn test_extract_exif_tiff_jpeg() {
        let tiff = build_tiff_le(&[(0x0112, 3, 1)]);
        let jpeg = jpeg_with_exif(&tiff);
        let extracted = extract_exif_tiff(&jpeg).unwrap();
        assert_eq!(extracted, tiff);
        // No EXIF case
        let no_exif = vec![0xFF, 0xD8, 0xFF, 0xD9];
        assert!(extract_exif_tiff(&no_exif).is_none());
    }

    #[test]
    fn test_extract_exif_tiff_png() {
        let tiff = build_tiff_le(&[(0x0112, 3, 1)]);
        let png = png_with_exif_chunk(&tiff);
        let extracted = extract_exif_tiff(&png).unwrap();
        assert_eq!(extracted, tiff);
    }

    #[test]
    fn test_extract_exif_tiff_webp() {
        let tiff = build_tiff_le(&[(0x0112, 3, 1)]);
        let webp = webp_with_exif_chunk(&tiff);
        let extracted = extract_exif_tiff(&webp).unwrap();
        assert_eq!(extracted, tiff);
    }

    #[test]
    fn test_extract_exif_tiff_tiff() {
        let tiff = build_tiff_le(&[(0x0112, 3, 1)]);
        let extracted = extract_exif_tiff(&tiff).unwrap();
        assert_eq!(extracted, tiff);
    }
}
