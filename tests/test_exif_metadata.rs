// Test bat_img_rs::exif::metadata
// Copyright © 2026 - Present, John Liu

mod common;

#[cfg(test)]
mod tests {
    use super::common::{
        build_minimal_tiff, build_tiff_le, build_tiff_with_exif_color_space, build_tiff_with_gps,
        find_color_space_value, jpeg_with_exif, png_with_exif_chunk, webp_with_exif_chunk,
    };
    use bat_img_rs::exif::{
        extract_exif_tiff, inject_exif_into_tiff, is_png, is_tiff, parse_exif_bytes, read_exif,
        rewrite_exif_metadata, strip_all_metadata, strip_gps_from_tiff, strip_gps_metadata,
        strip_tiff_metadata, write_exif_file,
    };
    use image::{GrayImage, ImageFormat, RgbImage};
    use tempfile::TempDir;

    // ---- TIFF construction / inspection helpers for the grayscale tests ------

    /// Value of an entry in the little-endian test builder below.
    enum Val {
        /// Stored in the 4-byte value field.
        Inline([u8; 4]),
        /// Stored out of line, right after the IFD that owns the entry.
        Data(Vec<u8>),
        /// Offset of another IFD (index into the slice given to `build_le_tiff`).
        Ifd(usize),
    }

    struct Ent {
        tag: u16,
        typ: u16,
        count: u32,
        val: Val,
    }

    fn ent(tag: u16, typ: u16, count: u32, val: Val) -> Ent {
        Ent {
            tag,
            typ,
            count,
            val,
        }
    }

    fn short(v: u16) -> Val {
        let b = v.to_le_bytes();
        Val::Inline([b[0], b[1], 0, 0])
    }

    fn long(v: u32) -> Val {
        Val::Inline(v.to_le_bytes())
    }

    fn shorts(vals: &[u16]) -> Val {
        Val::Data(vals.iter().flat_map(|v| v.to_le_bytes()).collect())
    }

    fn rationals(vals: &[(u32, u32)]) -> Val {
        Val::Data(
            vals.iter()
                .flat_map(|(n, d)| n.to_le_bytes().into_iter().chain(d.to_le_bytes()))
                .collect(),
        )
    }

    /// Little-endian TIFF: header, then every IFD immediately followed by its
    /// out-of-line values.  Entries must be given in ascending tag order.
    fn build_le_tiff(ifds: &[Vec<Ent>]) -> Vec<u8> {
        let mut offsets = Vec::new();
        let mut pos = 8usize;
        for ifd in ifds {
            offsets.push(pos);
            pos += 2 + ifd.len() * 12 + 4;
            for e in ifd {
                if let Val::Data(d) = &e.val {
                    pos += d.len() + d.len() % 2;
                }
            }
        }

        let mut out = Vec::with_capacity(pos);
        out.extend_from_slice(b"II\x2A\x00");
        out.extend_from_slice(&8u32.to_le_bytes());
        for ifd in ifds {
            out.extend_from_slice(&(ifd.len() as u16).to_le_bytes());
            let data_start = out.len() + ifd.len() * 12 + 4;
            let mut data: Vec<u8> = Vec::new();
            for e in ifd {
                out.extend_from_slice(&e.tag.to_le_bytes());
                out.extend_from_slice(&e.typ.to_le_bytes());
                out.extend_from_slice(&e.count.to_le_bytes());
                match &e.val {
                    Val::Inline(v) => out.extend_from_slice(v),
                    Val::Ifd(i) => out.extend_from_slice(&(offsets[*i] as u32).to_le_bytes()),
                    Val::Data(d) => {
                        out.extend_from_slice(&((data_start + data.len()) as u32).to_le_bytes());
                        data.extend_from_slice(d);
                        if !d.len().is_multiple_of(2) {
                            data.push(0);
                        }
                    }
                }
            }
            out.extend_from_slice(&0u32.to_le_bytes()); // next IFD
            out.extend_from_slice(&data);
        }
        out
    }

    /// A *colour* TIFF as an editor would write it: full pixel-layout tags for a
    /// 3-channel image, an RGB ICC profile, an RGB white point, and an Exif
    /// sub-IFD that in turn points at an Interoperability IFD.
    /// The pixel bytes themselves are appended so the file looks like a real image.
    fn build_colour_tiff_source(icc: &[u8]) -> Vec<u8> {
        let mut tiff = build_le_tiff(&[
            vec![
                ent(0x0100, 4, 1, long(64)),                       // ImageWidth
                ent(0x0101, 4, 1, long(48)),                       // ImageLength
                ent(0x0102, 3, 3, shorts(&[8, 8, 8])),             // BitsPerSample
                ent(0x0103, 3, 1, short(1)),                       // Compression
                ent(0x0106, 3, 1, short(2)),                       // Photometric = RGB
                ent(0x010F, 2, 6, Val::Data(b"Apple\0".to_vec())), // Make
                ent(0x0111, 4, 1, long(1000)),                     // StripOffsets
                ent(0x0112, 3, 1, short(6)),                       // Orientation
                ent(0x0115, 3, 1, short(3)),                       // SamplesPerPixel
                ent(0x0116, 4, 1, long(48)),                       // RowsPerStrip
                ent(0x0117, 4, 1, long(9216)),                     // StripByteCounts
                ent(0x011C, 3, 1, short(1)),                       // PlanarConfiguration
                ent(0x013E, 5, 2, rationals(&[(3127, 10000), (3290, 10000)])), // WhitePoint
                ent(0x0142, 3, 1, short(16)),                      // TileWidth
                ent(0x0143, 3, 1, short(16)),                      // TileLength
                ent(0x0153, 3, 3, shorts(&[1, 1, 1])),             // SampleFormat (3 samples)
                ent(0x8769, 4, 1, Val::Ifd(1)),                    // ExifOffset
                ent(0x8773, 7, icc.len() as u32, Val::Data(icc.to_vec())), // ICC profile
            ],
            vec![
                ent(0x9000, 7, 4, Val::Inline(*b"0232")), // ExifVersion
                ent(0xA001, 3, 1, short(1)),              // ColorSpace = sRGB
                ent(0xA005, 4, 1, Val::Ifd(2)),           // InteroperabilityOffset
            ],
            vec![
                ent(0x0001, 2, 4, Val::Inline(*b"R98\0")), // InteroperabilityIndex
                ent(0x0002, 7, 4, Val::Inline(*b"0100")),  // InteroperabilityVersion
            ],
        ]);
        tiff.extend(std::iter::repeat_n(0xABu8, 9216));
        tiff
    }

    fn fake_icc_profile() -> Vec<u8> {
        b"FAKE-RGB-ICC-PROFILE".repeat(32)
    }

    fn read_u16(b: &[u8], o: usize, le: bool) -> u16 {
        let s = [b[o], b[o + 1]];
        if le {
            u16::from_le_bytes(s)
        } else {
            u16::from_be_bytes(s)
        }
    }

    fn read_u32(b: &[u8], o: usize, le: bool) -> u32 {
        let s = [b[o], b[o + 1], b[o + 2], b[o + 3]];
        if le {
            u32::from_le_bytes(s)
        } else {
            u32::from_be_bytes(s)
        }
    }

    type RawEntry = [u8; 12];

    /// Byte-order aware read-only view over a TIFF file.
    struct TiffView<'a> {
        bytes: &'a [u8],
        le: bool,
    }

    impl<'a> TiffView<'a> {
        fn new(bytes: &'a [u8]) -> Self {
            assert!(is_tiff(bytes), "not a TIFF");
            Self {
                bytes,
                le: &bytes[0..2] == b"II",
            }
        }

        fn ifd0(&self) -> usize {
            read_u32(self.bytes, 4, self.le) as usize
        }

        fn entries(&self, ifd: usize) -> Vec<RawEntry> {
            let n = read_u16(self.bytes, ifd, self.le) as usize;
            (0..n)
                .map(|i| {
                    let o = ifd + 2 + i * 12;
                    let mut e = [0u8; 12];
                    e.copy_from_slice(&self.bytes[o..o + 12]);
                    e
                })
                .collect()
        }

        fn find(&self, ifd: usize, tag: u16) -> Option<RawEntry> {
            self.entries(ifd).into_iter().find(|e| self.tag(e) == tag)
        }

        fn tag(&self, e: &RawEntry) -> u16 {
            read_u16(e, 0, self.le)
        }

        fn typ(&self, e: &RawEntry) -> u16 {
            read_u16(e, 2, self.le)
        }

        fn count(&self, e: &RawEntry) -> u32 {
            read_u32(e, 4, self.le)
        }

        /// The raw 4-byte value field (an inline value, or an offset).
        fn value(&self, e: &RawEntry) -> u32 {
            read_u32(e, 8, self.le)
        }

        /// Every IFD reachable from IFD0 through Exif/GPS/Interop pointers and
        /// next-IFD links.  Panics if any pointer leads outside the file.
        fn reachable_ifds(&self) -> Vec<usize> {
            let mut seen: Vec<usize> = Vec::new();
            let mut todo = vec![self.ifd0()];
            while let Some(off) = todo.pop() {
                if seen.contains(&off) {
                    continue;
                }
                assert!(
                    off + 2 <= self.bytes.len(),
                    "IFD offset {off} is outside the file"
                );
                let n = read_u16(self.bytes, off, self.le) as usize;
                assert!(
                    off + 2 + n * 12 + 4 <= self.bytes.len(),
                    "IFD at {off} with {n} entries runs past the end of the file"
                );
                seen.push(off);
                for e in self.entries(off) {
                    if [0x8769u16, 0x8825, 0xA005].contains(&self.tag(&e)) {
                        todo.push(self.value(&e) as usize);
                    }
                }
                let next = read_u32(self.bytes, off + 2 + n * 12, self.le) as usize;
                if next != 0 {
                    todo.push(next);
                }
            }
            seen
        }

        /// TIFF 6.0: IFDs and out-of-line values start on a word boundary.  Every
        /// reachable IFD is checked; out-of-line values are checked only when they
        /// were written at or after `grafted_from` (values the encoder wrote earlier
        /// are the encoder's business, not the graft's).
        fn assert_word_aligned(&self, grafted_from: usize) {
            for ifd in self.reachable_ifds() {
                assert_eq!(ifd % 2, 0, "IFD at odd offset {ifd}");
                for e in self.entries(ifd) {
                    let unit = match self.typ(&e) {
                        1 | 2 | 6 | 7 => 1,
                        3 | 8 => 2,
                        4 | 9 | 11 => 4,
                        5 | 10 | 12 => 8,
                        _ => 0,
                    };
                    let total = self.count(&e) as usize * unit;
                    let off = self.value(&e) as usize;
                    if total > 4 && off >= grafted_from {
                        assert_eq!(
                            off % 2,
                            0,
                            "value of tag {:#06x} at odd offset {off}",
                            self.tag(&e)
                        );
                        assert!(
                            off + total <= self.bytes.len(),
                            "value of tag {:#06x} runs past the end of the file",
                            self.tag(&e)
                        );
                    }
                }
            }
        }
    }

    /// Tags that describe how the pixels of *this* file are stored.  Whatever
    /// the encoder wrote (or did not write) must survive the EXIF graft
    /// untouched, whatever the metadata source looked like.
    const PIXEL_LAYOUT_TAGS: &[u16] = &[
        0x0100, 0x0101, 0x0102, 0x0103, 0x0106, 0x0111, 0x0115, 0x0116, 0x0117, 0x011C, 0x0142,
        0x0143, 0x0153,
    ];

    fn assert_pixel_layout_unchanged(encoded: &[u8], grafted: &[u8]) {
        let (before, after) = (TiffView::new(encoded), TiffView::new(grafted));
        for &tag in PIXEL_LAYOUT_TAGS {
            assert_eq!(
                after.find(after.ifd0(), tag),
                before.find(before.ifd0(), tag),
                "pixel-layout tag {tag:#06x} must keep the encoder's value"
            );
        }
    }

    fn test_gray_image() -> GrayImage {
        GrayImage::from_fn(33, 21, |x, y| image::Luma([((x * 7 + y * 13) % 251) as u8]))
    }

    /// Regression test: when the source is a TIFF that carries a full image
    /// (strips + pixels), `rewrite_exif_metadata` must graft only the metadata
    /// tags and must NOT append the source's pixel data to the output.
    #[test]
    fn rewrite_exif_metadata_drops_tiff_pixel_data() {
        let tmp = TempDir::new().unwrap();
        let src_path = tmp.path().join("src.tiff");

        // A 100×100 RGB TIFF: pixel data dominates the file size.
        let mut img = RgbImage::new(100, 100);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgb([200, 100, 50]);
        }
        image::DynamicImage::ImageRgb8(img)
            .save_with_format(&src_path, image::ImageFormat::Tiff)
            .unwrap();
        let src_bytes = std::fs::read(&src_path).unwrap();

        // Graft the Make-bearing metadata TIFF into the source so that there is
        // at least one non-pixel tag for the rewrite step to carry over.
        let meta_source = build_tiff_with_gps(0x1234);
        let src_with_make = inject_exif_into_tiff(&src_bytes, &meta_source).unwrap();
        assert!(src_with_make.len() > 10_000, "source should be pixel-heavy");
        assert_eq!(
            parse_exif_bytes(&src_with_make).unwrap().make.as_deref(),
            Some("Apple")
        );

        // Minimal output TIFF: no pixels, no metadata.
        let output = build_minimal_tiff();
        let result = rewrite_exif_metadata(&output, &src_with_make, false).unwrap();

        // The result must be much smaller than the source, proving that the
        // source's strips were not appended.  This is the actual regression.
        assert!(
            result.len() < src_with_make.len(),
            "result {} should not have inherited source pixel data {}",
            result.len(),
            src_with_make.len()
        );

        // And the Make tag should still have been grafted.
        let info_after = parse_exif_bytes(&result).unwrap();
        assert_eq!(info_after.make.as_deref(), Some("Apple"));
    }

    #[test]
    fn strip_tiff_metadata_removes_exif_and_preserves_tiff() {
        let source = build_tiff_with_gps(0x1234);

        let stripped = strip_tiff_metadata(&source).unwrap();

        assert!(is_tiff(&stripped));
        assert_eq!(stripped.len(), source.len());

        let info = parse_exif_bytes(&stripped).unwrap();
        assert_eq!(info.make, None);
        assert!(!info.gps_present);
    }

    #[test]
    fn strip_all_metadata_uses_tiff_metadata_stripper() {
        let source = build_tiff_with_gps(0x5678);

        let stripped = strip_all_metadata(&source).unwrap();

        assert!(is_tiff(&stripped));
        let info = parse_exif_bytes(&stripped).unwrap();
        assert_eq!(info.make, None);
        assert!(!info.gps_present);
    }

    #[test]
    fn strip_tiff_metadata_preserves_pixels() {
        let original =
            image::GrayImage::from_fn(17, 11, |x, y| image::Luma([((x * 13 + y * 7) % 251) as u8]));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.tiff");
        original.save_with_format(&path, ImageFormat::Tiff).unwrap();

        let encoded = std::fs::read(&path).unwrap();
        let with_exif =
            bat_img_rs::exif::inject_exif_into_tiff(&encoded, &build_tiff_with_gps(0x1234))
                .unwrap();

        let stripped = strip_tiff_metadata(&with_exif).unwrap();
        std::fs::write(&path, stripped).unwrap();

        let decoded = image::open(&path).unwrap().to_luma8();
        assert_eq!(decoded.dimensions(), original.dimensions());
        assert_eq!(decoded.as_raw(), original.as_raw());
    }

    #[test]
    fn strip_tiff_metadata_is_idempotent() {
        let source = build_tiff_with_gps(0x1234);

        let once = strip_tiff_metadata(&source).unwrap();
        let twice = strip_tiff_metadata(&once).unwrap();

        assert_eq!(once, twice);
    }

    #[test]
    fn strip_tiff_metadata_passes_non_tiff_data_through() {
        let data = b"not a TIFF";
        assert_eq!(strip_tiff_metadata(data).unwrap(), data);
    }

    // ---- grayscale TIFF regression tests -------------------------------------

    #[test]
    fn test_grayscale_tiff_color_space_long_not_garbled() {
        let output = build_minimal_tiff();
        // ColorSpace as LONG (type 4) with value 1 (sRGB)
        let source = build_tiff_with_exif_color_space(4, 1);

        let result = rewrite_exif_metadata(&output, &source, true).unwrap();

        assert_eq!(
            find_color_space_value(&result),
            Some(0xFFFF),
            "LONG ColorSpace must become 0xFFFF, not a huge bogus value"
        );
    }

    #[test]
    fn test_grayscale_tiff_color_space_short() {
        let output = build_minimal_tiff();
        // Standard SHORT (type 3) ColorSpace
        let source = build_tiff_with_exif_color_space(3, 1);

        let result = rewrite_exif_metadata(&output, &source, true).unwrap();

        assert_eq!(find_color_space_value(&result), Some(0xFFFF));
    }

    #[test]
    fn test_grayscale_tiff_color_space_absent_is_safe() {
        // Source without any Exif sub‑IFD
        let output = build_minimal_tiff();
        let source = build_minimal_tiff();

        let result = rewrite_exif_metadata(&output, &source, true).unwrap();

        // No ColorSpace tag should be present, and the output must remain a valid TIFF
        assert!(is_tiff(&result));
        assert_eq!(find_color_space_value(&result), None);
    }

    #[test]
    fn test_grayscale_tiff_orientation_long_not_garbled() {
        // Build a TIFF whose IFD0 contains Orientation as LONG (type 4)
        let mut t = Vec::new();
        t.extend_from_slice(b"II\x2A\x00");
        t.extend_from_slice(&8u32.to_le_bytes());
        t.extend_from_slice(&1u16.to_le_bytes()); // 1 entry
        t.extend_from_slice(&0x0112u16.to_le_bytes()); // Orientation
        t.extend_from_slice(&4u16.to_le_bytes()); // LONG
        t.extend_from_slice(&1u32.to_le_bytes());
        t.extend_from_slice(&6u32.to_le_bytes()); // original value 6
        t.extend_from_slice(&0u32.to_le_bytes());

        let result = rewrite_exif_metadata(&t, &t, true).unwrap();

        // Orientation should be reset to 1 (0x00000001), not a huge number
        let le = &result[0..2] == b"II";
        let ru32 = |o: usize| {
            if le {
                u32::from_le_bytes([result[o], result[o + 1], result[o + 2], result[o + 3]])
            } else {
                u32::from_be_bytes([result[o], result[o + 1], result[o + 2], result[o + 3]])
            }
        };
        let ifd0 = ru32(4) as usize;
        let cnt = u16::from_le_bytes([result[ifd0], result[ifd0 + 1]]) as usize;
        let mut found = None;
        for i in 0..cnt {
            let e = ifd0 + 2 + i * 12;
            if u16::from_le_bytes([result[e], result[e + 1]]) == 0x0112 {
                found = Some(ru32(e + 8));
            }
        }
        assert_eq!(found, Some(1));
    }

    #[test]
    fn test_write_exif_file_grayscale_tiff_end_to_end() {
        let tmp = TempDir::new().unwrap();
        let output_path = tmp.path().join("gray.tiff");

        // Encoded output TIFF (no EXIF)
        let encoded = build_minimal_tiff();
        std::fs::write(&output_path, &encoded).unwrap();

        // Source with EXIF + LONG ColorSpace
        let source = build_tiff_with_exif_color_space(4, 1);

        bat_img_rs::exif::write_exif_file(&output_path, &source, true).unwrap();

        let result = std::fs::read(&output_path).unwrap();
        assert!(is_tiff(&result));
        assert_eq!(find_color_space_value(&result), Some(0xFFFF));
    }

    /// The reported bug.  Grafting metadata from a *colour* TIFF onto a
    /// grayscale TIFF used to copy the source's RGB ICC profile, white point
    /// and multi-sample layout tags into the 1-channel file, which viewers then
    /// render as garbage.  The pixels and the pixel-layout tags of the encoded
    /// file must stay exactly as the encoder wrote them.
    #[test]
    fn test_write_exif_file_grayscale_tiff_from_colour_tiff_source_is_not_garbled() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("gray.tiff");
        let gray = test_gray_image();
        gray.save_with_format(&path, ImageFormat::Tiff).unwrap();
        let encoded = std::fs::read(&path).unwrap();

        let source = build_colour_tiff_source(&fake_icc_profile());
        write_exif_file(&path, &source, true).unwrap();
        let result = std::fs::read(&path).unwrap();

        // The image still decodes to exactly the pixels that were encoded.
        let decoded = image::open(&path).unwrap().to_luma8();
        assert_eq!(decoded.dimensions(), gray.dimensions());
        assert_eq!(decoded.as_raw(), gray.as_raw());

        // No colour-model or pixel-layout description leaked in from the source.
        assert_pixel_layout_unchanged(&encoded, &result);
        let after = TiffView::new(&result);
        for tag in [0x8773u16, 0x013E, 0x013F, 0x012D] {
            assert!(
                after.find(after.ifd0(), tag).is_none(),
                "colour tag {tag:#06x} from the source must not be attached to a grayscale image"
            );
        }

        // ...while the real metadata was still carried over.
        let info = parse_exif_bytes(&result).unwrap();
        assert_eq!(info.make.as_deref(), Some("Apple"));
        assert_eq!(find_color_space_value(&result), Some(0xFFFF));
        after.assert_word_aligned(encoded.len());
    }

    /// Guard against over-filtering: a colour output keeps the colour profile
    /// of a colour source, and its own pixel layout is still left alone.
    #[test]
    fn test_write_exif_file_colour_tiff_keeps_colour_profile() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("colour.tiff");
        let rgb = RgbImage::from_fn(33, 21, |x, y| {
            image::Rgb([(x * 7) as u8, (y * 11) as u8, ((x + y) * 5) as u8])
        });
        rgb.save_with_format(&path, ImageFormat::Tiff).unwrap();
        let encoded = std::fs::read(&path).unwrap();

        let icc = fake_icc_profile();
        let source = build_colour_tiff_source(&icc);
        write_exif_file(&path, &source, false).unwrap();
        let result = std::fs::read(&path).unwrap();

        let decoded = image::open(&path).unwrap().to_rgb8();
        assert_eq!(decoded.as_raw(), rgb.as_raw());
        assert_pixel_layout_unchanged(&encoded, &result);

        let after = TiffView::new(&result);
        let profile = after
            .find(after.ifd0(), 0x8773)
            .expect("ICC profile of a colour source must be kept for a colour image");
        let off = after.value(&profile) as usize;
        assert_eq!(&result[off..off + icc.len()], icc.as_slice());
        assert!(after.find(after.ifd0(), 0x013E).is_some());
        assert_eq!(find_color_space_value(&result), Some(1));
        after.assert_word_aligned(encoded.len());
    }

    /// Metadata from a JPEG (big-endian EXIF) grafted onto a real grayscale
    /// TIFF (little-endian) goes through the endianness-conversion path.
    #[test]
    fn test_write_exif_file_grayscale_tiff_from_jpeg_source_keeps_pixels() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("gray_from_jpeg.tiff");
        let gray = test_gray_image();
        gray.save_with_format(&path, ImageFormat::Tiff).unwrap();
        let encoded = std::fs::read(&path).unwrap();

        let jpeg = jpeg_with_exif(&build_tiff_with_gps(0x1234));
        let source = strip_gps_metadata(&jpeg).unwrap();
        write_exif_file(&path, &source, true).unwrap();
        let result = std::fs::read(&path).unwrap();

        let decoded = image::open(&path).unwrap().to_luma8();
        assert_eq!(decoded.as_raw(), gray.as_raw());
        assert_pixel_layout_unchanged(&encoded, &result);

        let info = parse_exif_bytes(&result).unwrap();
        assert_eq!(info.make.as_deref(), Some("Apple"));
        assert!(!info.gps_present);
    }

    /// The Interoperability IFD pointer (0xA005, inside the Exif sub-IFD) has to
    /// be relocated together with the grafted blob.  Left alone it pointed into
    /// the image's pixel data.
    #[test]
    fn test_grayscale_tiff_interop_ifd_pointer_is_relocated() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("gray_interop.tiff");
        test_gray_image()
            .save_with_format(&path, ImageFormat::Tiff)
            .unwrap();
        let encoded = std::fs::read(&path).unwrap();

        let source = build_colour_tiff_source(&fake_icc_profile());
        let result = rewrite_exif_metadata(&encoded, &source, true).unwrap();

        let v = TiffView::new(&result);
        let exif = v
            .find(v.ifd0(), 0x8769)
            .expect("ExifOffset must be present");
        let exif_off = v.value(&exif) as usize;
        assert!(
            exif_off >= encoded.len(),
            "Exif IFD at {exif_off} must be in the appended region, not inside the image"
        );

        let interop = v
            .find(exif_off, 0xA005)
            .expect("Interoperability pointer must be present in the Exif IFD");
        let interop_off = v.value(&interop) as usize;
        assert!(
            interop_off >= encoded.len(),
            "Interop IFD at {interop_off} must be in the appended region, not inside the image"
        );

        let tags: Vec<u16> = v.entries(interop_off).iter().map(|e| v.tag(e)).collect();
        assert_eq!(tags, vec![0x0001, 0x0002]);
        let index = v.find(interop_off, 0x0001).unwrap();
        assert_eq!(index[8..12].to_vec(), b"R98\0".to_vec());

        v.assert_word_aligned(encoded.len());
    }

    /// An encoder may leave the TIFF with an odd length.  The appended IFD0 must
    /// still start on a word boundary, and so must everything grafted with it.
    #[test]
    fn test_inject_exif_into_odd_length_tiff_is_word_aligned() {
        let mut output = build_minimal_tiff();
        output.push(0); // odd length
        assert_eq!(output.len() % 2, 1);

        let source = build_colour_tiff_source(&fake_icc_profile());
        let result = rewrite_exif_metadata(&output, &source, true).unwrap();

        assert!(result.len() > output.len());
        assert_eq!(TiffView::new(&result).ifd0() % 2, 0);
        TiffView::new(&result).assert_word_aligned(output.len());
    }

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
