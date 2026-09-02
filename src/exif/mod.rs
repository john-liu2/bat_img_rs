#![allow(unused_imports)] // re‑exports are intended for external use

//! EXIF metadata for HEIC, JPEG, PNG, TIFF, WebP
//!
//! GPS removal rewrites the embedded TIFF/EXIF block in-place where possible.
//! Orientation is read from the same EXIF block across all supported containers.

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
    rewrite_exif_metadata, strip_all_metadata, strip_gps_from_tiff, strip_gps_metadata,
    write_exif_file,
};
pub use parser::{ExifInfo, parse_exif_bytes, read_exif, read_orientation};
