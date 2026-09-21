// re‑exports are intended for external use
// Copyright © 2026 - Present, John Liu

#![allow(unused_imports)]

mod container;
mod heic;
mod icc;
mod image_details;
mod metadata;
mod parser;

pub use container::{extract_exif_tiff, is_jpeg, is_png, is_tiff, is_webp};
pub use heic::{extract_heic_exif_raw, replace_heic_exif_payload, tiff_from_heic_metadata};
pub use icc::get_icc_profile_name;
pub use image_details::{ImageDetails, get_image_details};
pub use metadata::{
    inject_exif_into_tiff, rewrite_exif_metadata, strip_all_metadata, strip_gps_from_tiff,
    strip_gps_metadata, strip_tiff_metadata, write_exif_file,
};
pub use parser::{ExifInfo, parse_exif_bytes, read_exif, read_orientation};
