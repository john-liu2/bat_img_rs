use thiserror::Error;

#[derive(Error, Debug)]
pub enum BatImgError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Image decode/encode error: {0}")]
    Image(#[from] image::ImageError),

    #[error("Invalid resize spec '{0}': expected WxH (e.g. 1920x1080 or 1920x0)")]
    InvalidResize(String),

    #[error("Invalid rotation '{0}': must be 90, 180, or 270")]
    InvalidRotation(u32),

    #[error("Invalid color '{0}'")]
    InvalidColor(String),

    #[error("EXIF processing error: {0}")]
    Exif(String),

    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),
}
