/// Unit tests for bat_img_rs::pipeline
#[cfg(test)]
mod tests {
    use bat_img_rs::pipeline::parse_color;
    use image::Rgba;

    // ── parse_color ───────────────────────────────────────────────────────────

    #[test]
    fn color_named_white() {
        assert_eq!(parse_color("white").unwrap(), Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn color_named_black() {
        assert_eq!(parse_color("black").unwrap(), Rgba([0, 0, 0, 255]));
    }

    #[test]
    fn color_named_red() {
        assert_eq!(parse_color("red").unwrap(), Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn color_named_green() {
        assert_eq!(parse_color("green").unwrap(), Rgba([0, 128, 0, 255]));
    }

    #[test]
    fn color_named_blue() {
        assert_eq!(parse_color("blue").unwrap(), Rgba([0, 0, 255, 255]));
    }

    #[test]
    fn color_named_gray() {
        assert_eq!(parse_color("gray").unwrap(), Rgba([128, 128, 128, 255]));
    }

    #[test]
    fn color_named_grey_alias() {
        assert_eq!(parse_color("grey").unwrap(), parse_color("gray").unwrap());
    }

    #[test]
    fn color_named_transparent() {
        assert_eq!(parse_color("transparent").unwrap(), Rgba([0, 0, 0, 0]));
    }

    #[test]
    fn color_named_case_insensitive() {
        assert_eq!(parse_color("WHITE").unwrap(), Rgba([255, 255, 255, 255]));
        assert_eq!(parse_color("White").unwrap(), Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn color_hex_six_digits() {
        assert_eq!(
            parse_color("#ff8800").unwrap(),
            Rgba([0xFF, 0x88, 0x00, 255])
        );
    }

    #[test]
    fn color_hex_six_uppercase() {
        assert_eq!(
            parse_color("#FF8800").unwrap(),
            Rgba([0xFF, 0x88, 0x00, 255])
        );
    }

    #[test]
    fn color_hex_eight_digits_with_alpha() {
        assert_eq!(
            parse_color("#ff880080").unwrap(),
            Rgba([0xFF, 0x88, 0x00, 0x80])
        );
    }

    #[test]
    fn color_hex_full_opaque_white() {
        assert_eq!(parse_color("#ffffff").unwrap(), Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn color_hex_full_transparent() {
        assert_eq!(parse_color("#00000000").unwrap(), Rgba([0, 0, 0, 0]));
    }

    #[test]
    fn color_invalid_name_errors() {
        assert!(parse_color("mauve").is_err());
        assert!(parse_color("").is_err());
        assert!(parse_color("rgb(255,0,0)").is_err());
    }

    #[test]
    fn color_invalid_hex_length_errors() {
        assert!(parse_color("#fff").is_err()); // 3 digits
        assert!(parse_color("#ffff").is_err()); // 4 digits
        assert!(parse_color("#fffff").is_err()); // 5 digits
        assert!(parse_color("#fffffff").is_err()); // 7 digits
    }

    #[test]
    fn color_invalid_hex_chars_errors() {
        assert!(parse_color("#zzzzzz").is_err());
    }

    // ── resize spec parsing ───────────────────────────────────────────────────

    #[test]
    fn resize_spec_width_only() {
        let (w, h) = bat_img_rs::processor::resolve_dimensions(3840, 2160, 1920, 0);
        assert_eq!(w, 1920);
        assert_eq!(h, 1080);
    }

    #[test]
    fn resize_spec_height_only() {
        let (w, h) = bat_img_rs::processor::resolve_dimensions(1920, 1080, 0, 1080);
        assert_eq!(w, 1920);
        assert_eq!(h, 1080);
    }

    #[test]
    fn resize_spec_both_dimensions() {
        let (w, h) = bat_img_rs::processor::resolve_dimensions(4000, 3000, 800, 600);
        assert_eq!(w, 800);
        assert_eq!(h, 600);
    }

    #[test]
    fn resize_spec_zero_zero_passthrough() {
        let (w, h) = bat_img_rs::processor::resolve_dimensions(1920, 1080, 0, 0);
        assert_eq!(w, 1920);
        assert_eq!(h, 1080);
    }

    #[test]
    fn resize_spec_width_rounding() {
        let (w, h) = bat_img_rs::processor::resolve_dimensions(100, 75, 0, 50);
        assert_eq!(h, 50);
        assert_eq!(w, 67);
    }

    #[test]
    fn resize_spec_height_rounding() {
        let (w, h) = bat_img_rs::processor::resolve_dimensions(100, 75, 50, 0);
        assert_eq!(w, 50);
        assert_eq!(h, 38);
    }

    #[test]
    fn resize_spec_square_image() {
        let (w, h) = bat_img_rs::processor::resolve_dimensions(500, 500, 200, 0);
        assert_eq!(w, 200);
        assert_eq!(h, 200);
    }
}
