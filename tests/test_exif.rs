/// Unit tests for bat_img_rs::exif
///
/// All tests synthesise minimal valid JPEG / TIFF byte sequences in memory —
/// no fixture files are required.
#[cfg(test)]
mod tests {
    use bat_img_rs::exif::{
        extract_exif_tiff, graft_exif_metadata, is_png, is_tiff, read_orientation, strip_all_metadata,
        strip_gps_metadata,
    };
    use image::RgbImage;
    use tempfile::TempDir;

    // ── TIFF / EXIF byte-building helpers ─────────────────────────────────────

    fn png_with_exif_chunk(exif: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"\x89PNG\r\n\x1a\n");

        // eXIf chunk
        out.extend_from_slice(&(exif.len() as u32).to_be_bytes());
        out.extend_from_slice(b"eXIf");
        out.extend_from_slice(exif);

        // Fake CRC (not needed by your chunk rewriter)
        out.extend_from_slice(&0u32.to_be_bytes());

        // IEND chunk
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(b"IEND");
        out.extend_from_slice(&0u32.to_be_bytes());
        out
    }

    fn webp_with_exif_chunk(exif: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();

        body.extend_from_slice(b"EXIF");
        body.extend_from_slice(&(exif.len() as u32).to_le_bytes());
        body.extend_from_slice(exif);

        if exif.len() % 2 == 1 {
            body.push(0);
        }
        let file_size = 4 + body.len();
        let mut out = Vec::new();

        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(file_size as u32).to_le_bytes());
        out.extend_from_slice(b"WEBP");
        out.extend_from_slice(&body);
        out
    }

    /// Build a minimal little-endian TIFF block containing a single IFD with
    /// the given tag entries.  Each entry is `(tag, type, value_u16)`.
    fn build_tiff_le(entries: &[(u16, u16, u16)]) -> Vec<u8> {
        // TIFF header: "II" + magic 42 (LE) + IFD offset = 8
        let mut buf: Vec<u8> = vec![
            b'I', b'I', // little-endian
            0x2A, 0x00, // magic
            0x08, 0x00, 0x00, 0x00, // IFD at offset 8
        ];

        // IFD: entry count (u16 LE)
        let count = entries.len() as u16;
        buf.extend_from_slice(&count.to_le_bytes());

        for &(tag, typ, val) in entries {
            buf.extend_from_slice(&tag.to_le_bytes()); // tag
            buf.extend_from_slice(&typ.to_le_bytes()); // type (3 = SHORT)
            buf.extend_from_slice(&1u32.to_le_bytes()); // count = 1
            // For SHORT values ≤ 4 bytes, the value is stored directly in the
            // value-offset field (little-endian, zero-padded).
            buf.extend_from_slice(&(val as u32).to_le_bytes());
        }

        // Next-IFD offset = 0 (no more IFDs)
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf
    }

    /// Build a minimal big-endian TIFF block with a single orientation entry.
    fn build_tiff_be(orientation: u16) -> Vec<u8> {
        let mut buf: Vec<u8> = vec![
            b'M', b'M', // big-endian
            0x00, 0x2A, // magic
            0x00, 0x00, 0x00, 0x08, // IFD at offset 8
        ];
        // 1 entry
        buf.extend_from_slice(&1u16.to_be_bytes());
        // tag 0x0112 = Orientation
        buf.extend_from_slice(&0x0112u16.to_be_bytes());
        buf.extend_from_slice(&3u16.to_be_bytes()); // SHORT
        buf.extend_from_slice(&1u32.to_be_bytes()); // count
        // TIFF spec: for a SHORT stored inline in the 4-byte value-offset field,
        // big-endian layout puts the value in the *first* 2 bytes of those 4 bytes.
        buf.extend_from_slice(&orientation.to_be_bytes()); // value (2 bytes)
        buf.extend_from_slice(&[0x00, 0x00]); // padding (2 bytes)
        buf.extend_from_slice(&0u32.to_be_bytes()); // next IFD
        buf
    }

    /// Wrap a TIFF block in a minimal JPEG APP1 EXIF segment.
    fn jpeg_with_exif(tiff: &[u8]) -> Vec<u8> {
        let exif_header = b"Exif\x00\x00";
        let payload_len = exif_header.len() + tiff.len();
        let seg_len = (payload_len + 2) as u16; // includes the length field

        let mut jpeg = vec![0xFF, 0xD8]; // SOI
        jpeg.push(0xFF);
        jpeg.push(0xE1); // APP1
        jpeg.extend_from_slice(&seg_len.to_be_bytes());
        jpeg.extend_from_slice(exif_header);
        jpeg.extend_from_slice(tiff);
        // Append a minimal SOS marker so the file looks structurally complete
        jpeg.extend_from_slice(&[0xFF, 0xD9]); // EOI
        jpeg
    }

    /// Build a TIFF with a GPS IFD pointer (tag 0x8825) set to a non-zero offset.
    fn build_tiff_with_gps(gps_offset: u32) -> Vec<u8> {
        let mut buf: Vec<u8> = vec![b'I', b'I', 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00];
        // 2 entries: Orientation + GPSInfoIFD
        buf.extend_from_slice(&2u16.to_le_bytes());

        // Orientation = 1
        buf.extend_from_slice(&0x0112u16.to_le_bytes());
        buf.extend_from_slice(&3u16.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());

        // GPSInfoIFDPointer = gps_offset
        buf.extend_from_slice(&0x8825u16.to_le_bytes());
        buf.extend_from_slice(&4u16.to_le_bytes()); // LONG
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&gps_offset.to_le_bytes());

        buf.extend_from_slice(&0u32.to_le_bytes()); // next IFD
        buf
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

        let grafted = graft_exif_metadata(&encoded, &stripped).unwrap();
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

        let grafted = graft_exif_metadata(&encoded, &stripped).unwrap();
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
