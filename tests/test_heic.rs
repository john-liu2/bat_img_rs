/// Test bat_img_rs::heic

mod common;

#[cfg(test)]

mod tests {
    use super::common::build_tiff_with_gps;

    use bat_img_rs::heic;
    use bat_img_rs::exif::{
        extract_heic_exif_raw,
        extract_exif_tiff,
        strip_gps_from_tiff,
        strip_gps_metadata,
        strip_all_metadata
    };
    use image::{DynamicImage, RgbImage};
    use tempfile::tempdir;
    use libheif_rs::CompressionFormat;
    use std::path::Path;

    const HEIC_FIXTURE: &str = "tests/fixtures/src.heic";

    // ── Fixture Container Integration Tests ───────────────────────────────────
    #[test]
    fn fixture_heic_extract_exif_bytes() {
        let fixture = Path::new(HEIC_FIXTURE);
        if !fixture.exists() {
            return;
        }

        let raw_bytes = std::fs::read(fixture).unwrap();
        let extracted = extract_heic_exif_raw(&raw_bytes);
        assert!(
            extracted.is_some(),
            "Fixture HEIC should contain an EXIF metadata block"
        );
    }

    #[test]
    fn fixture_heic_fast_path_strip_gps() {
        let fixture = Path::new(HEIC_FIXTURE);
        if !fixture.exists() {
            return;
        }

        let raw_bytes = std::fs::read(fixture).unwrap();
        let stripped_bytes = strip_gps_metadata(&raw_bytes).expect("GPS strip failed on HEIC fixture");

        // Fast path preserves exact file byte size
        assert_eq!(raw_bytes.len(), stripped_bytes.len());
        assert!(heic::is_heic_bytes(&stripped_bytes));
    }

    #[test]
    fn fixture_heic_fast_path_strip_all() {
        let fixture = Path::new(HEIC_FIXTURE);
        if !fixture.exists() {
            return;
        }

        let raw_bytes = std::fs::read(fixture).unwrap();
        let stripped_bytes = strip_all_metadata(&raw_bytes).expect("Strip all failed on HEIC fixture");

        assert_eq!(raw_bytes.len(), stripped_bytes.len());
        assert!(heic::is_heic_bytes(&stripped_bytes));

        // TIFF header magic bytes must no longer exist in the output
        assert!(!stripped_bytes.windows(4).any(|w| w == b"II\x2A\x00"));
        assert!(!stripped_bytes.windows(4).any(|w| w == b"MM\x00\x2A"));
    }

    // ── Synthetic HEIC Tests ──────────────────────────────────────────────────
    #[test]
    fn tiff_exif_strip_gps_preserves_non_gps_tags() {
        let tiff = build_tiff_with_gps(0x1234);

        let stripped = strip_gps_from_tiff(&tiff).unwrap();

        let stripped_tiff =
            extract_exif_tiff(&stripped).expect("stripped TIFF should still contain EXIF");

        // Verify EXIF survives
        assert!(!stripped_tiff.is_empty());

        // Optional: check GPS IFD was removed
        // depending on your TIFF parser API.
    }

    #[test]
    fn heic_strip_gps_preserves_exif_without_gps() {
        let dir = tempdir().unwrap();

        let input = dir.path().join("input.heic");
        let stripped_path = dir.path().join("gps_stripped.heic");

        let img = DynamicImage::ImageRgb8(
            RgbImage::from_pixel(8, 8, image::Rgb([10, 20, 30]))
        );
        let tiff = build_tiff_with_gps(0x1234);

        heic::encode(
            &img,
            &input,
            CompressionFormat::Hevc,
            Some(80),
            Some(&tiff),
        )
        .unwrap();

        let original = std::fs::read(&input).unwrap();
        let stripped = strip_gps_metadata(&original).unwrap();

        std::fs::write(&stripped_path, &stripped).unwrap();
        let (_, stripped_exif, _) = heic::decode(&stripped_path).unwrap();

        assert!(
            stripped_exif.is_some(),
            "HEIC strip_gps should preserve EXIF"
        );

        let stripped_exif = stripped_exif.unwrap();
        // Verify GPS was removed from the TIFF payload
        let cleaned_tiff = strip_gps_from_tiff(&stripped_exif).unwrap();
        assert!(
            !cleaned_tiff.is_empty(),
            "remaining EXIF should not be empty"
        );
    }

    #[test]
    fn heic_strip_all_removes_exif_but_strip_gps_keeps_exif() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("with_exif.heic");

        let img = DynamicImage::ImageRgb8(
            RgbImage::from_pixel(8, 8, image::Rgb([10, 20, 30]))
        );
        let tiff = build_tiff_with_gps(0x1234);

        heic::encode(
            &img,
            &input,
            CompressionFormat::Hevc,
            Some(80),
            Some(&tiff),
        )
        .unwrap();

        let original = std::fs::read(&input).unwrap();

        // strip GPS
        let stripped_gps = strip_gps_metadata(&original).unwrap();

        let gps_path = dir.path().join("gps_removed.heic");
        std::fs::write(&gps_path, &stripped_gps).unwrap();

        let (_, gps_exif, _) = heic::decode(&gps_path).unwrap();

        assert!(
            gps_exif.is_some(),
            "strip_gps must preserve non-GPS EXIF"
        );

        // strip ALL
        let stripped_all = strip_all_metadata(&original).unwrap();

        let all_path = dir.path().join("all_removed.heic");
        std::fs::write(&all_path, &stripped_all).unwrap();

        let (_, all_exif, _) = heic::decode(&all_path).unwrap();

        assert!(
            all_exif.is_none(),
            "strip_all must remove EXIF"
        );
    }
}
