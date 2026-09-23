// Test bat_img_rs::exif::extract_exif_tiff + parse_exif_bytes across every
// supported container: JPEG, PNG, WebP, TIFF and HEIC.
// Copyright © 2026 - Present, John Liu

mod common;

#[cfg(test)]
mod tests {
    use super::common::{
        build_tiff_with_gps, jpeg_with_exif, mock_heic_with_exif, png_with_exif_chunk,
        webp_with_exif_chunk,
    };
    use bat_img_rs::exif::{
        extract_exif_tiff, parse_exif_bytes, strip_all_metadata, strip_gps_metadata,
    };

    const PREFIX: &[u8] = b"Exif\0\0";

    fn sample_tiff() -> Vec<u8> {
        build_tiff_with_gps(0x1234).to_vec()
    }

    /// The same EXIF block wrapped in each container.
    fn all_formats(tiff: &[u8]) -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("jpeg", jpeg_with_exif(tiff)),
            ("png", png_with_exif_chunk(tiff)),
            ("webp", webp_with_exif_chunk(tiff)),
            ("tiff", tiff.to_vec()),
            ("heic", mock_heic_with_exif(tiff)),
        ]
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in data {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// True when every chunk of the PNG is complete and carries a correct CRC.
    fn png_crcs_valid(png: &[u8]) -> bool {
        let mut pos = 8;
        while pos + 12 <= png.len() {
            let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
            let end = pos + 12 + len;
            if end > png.len() {
                return false;
            }
            let stored = u32::from_be_bytes(png[end - 4..end].try_into().unwrap());
            if stored != crc32(&png[pos + 4..end - 4]) {
                return false;
            }
            pos = end;
        }
        pos == png.len()
    }

    #[test]
    fn extract_then_parse_gives_the_same_answer_for_every_format() {
        let tiff = sample_tiff();
        for (name, bytes) in all_formats(&tiff) {
            let extracted = extract_exif_tiff(&bytes)
                .unwrap_or_else(|| panic!("{name}: no EXIF block extracted"));
            assert!(
                extracted.starts_with(&tiff),
                "{name}: extracted block differs from the embedded one"
            );
            let info = parse_exif_bytes(&extracted).expect("parse_exif_bytes failed");
            assert_eq!(info.make.as_deref(), Some("Apple"), "{name}");
            assert!(info.gps_present, "{name}");
        }
    }

    #[test]
    fn images_without_exif_yield_none() {
        let no_exif: Vec<(&str, Vec<u8>)> = vec![
            ("jpeg", vec![0xFF, 0xD8, 0xFF, 0xD9]),
            (
                "png",
                [
                    &b"\x89PNG\r\n\x1a\n"[..],
                    &b"\x00\x00\x00\x00IEND\xAE\x42\x60\x82"[..],
                ]
                .concat(),
            ),
            ("webp", b"RIFF\x04\x00\x00\x00WEBP".to_vec()),
            ("heic", b"\x00\x00\x00\x10ftypheic\x00\x00\x00\x00".to_vec()),
            ("garbage", b"this is not an image".to_vec()),
            ("empty", Vec::new()),
        ];
        for (name, bytes) in no_exif {
            assert!(extract_exif_tiff(&bytes).is_none(), "{name}");
        }
    }

    #[test]
    fn every_prefix_of_every_format_is_handled_without_panicking() {
        let tiff = sample_tiff();
        for (name, bytes) in all_formats(&tiff) {
            for n in 0..=bytes.len() {
                let _ = extract_exif_tiff(&bytes[..n]);
            }
            // Only the whole file may yield the complete block.
            let whole = extract_exif_tiff(&bytes).unwrap_or_default();
            assert!(!whole.is_empty(), "{name}");
        }
        let heic = mock_heic_with_exif(&tiff);
        for n in 0..=heic.len() {
            let _ = strip_all_metadata(&heic[..n]);
        }
    }

    #[test]
    fn exif_prefix_inside_png_and_webp_chunks_is_dropped() {
        let tiff = sample_tiff();
        let mut prefixed = PREFIX.to_vec();
        prefixed.extend_from_slice(&tiff);

        assert_eq!(
            extract_exif_tiff(&png_with_exif_chunk(&prefixed)).unwrap(),
            tiff
        );
        assert_eq!(
            extract_exif_tiff(&webp_with_exif_chunk(&prefixed)).unwrap(),
            tiff
        );
    }

    #[test]
    fn jpeg_extraction_skips_xmp_and_fill_bytes() {
        fn app1(payload: &[u8]) -> Vec<u8> {
            let mut segment = vec![0xFF, 0xE1];
            segment.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
            segment.extend_from_slice(payload);
            segment
        }
        let tiff = sample_tiff();
        let mut exif_payload = PREFIX.to_vec();
        exif_payload.extend_from_slice(&tiff);

        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend(app1(b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta/>"));
        jpeg.push(0xFF); // fill byte before the next marker
        jpeg.extend(app1(&exif_payload));
        jpeg.extend([0xFF, 0xD9]);

        assert_eq!(extract_exif_tiff(&jpeg).unwrap(), tiff);
    }

    #[test]
    fn strip_all_metadata_removes_exif_from_every_container_format() {
        let tiff = sample_tiff();
        for (name, bytes) in all_formats(&tiff) {
            if name == "tiff" {
                continue; // a TIFF *is* its metadata; only GPS is removed
            }
            let stripped = strip_all_metadata(&bytes).unwrap();
            assert!(
                extract_exif_tiff(&stripped).is_none(),
                "{name}: EXIF survived strip_all_metadata"
            );
        }
    }

    #[test]
    fn strip_gps_keeps_the_rest_of_the_exif_in_every_format() {
        let tiff = sample_tiff();
        for (name, bytes) in all_formats(&tiff) {
            let stripped = strip_gps_metadata(&bytes).unwrap();

            let extracted = extract_exif_tiff(&stripped)
                .unwrap_or_else(|| panic!("{name}: EXIF lost while stripping GPS"));
            let info = parse_exif_bytes(&extracted).unwrap();
            assert_eq!(info.make.as_deref(), Some("Apple"), "{name}");
            assert!(!info.gps_present, "{name}: GPS survived");

            if name == "png" && png_crcs_valid(&bytes) {
                assert!(png_crcs_valid(&stripped), "png: CRC broken by rewrite");
            }
        }
    }
}
